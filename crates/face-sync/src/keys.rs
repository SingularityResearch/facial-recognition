//! Keys kept in the shared database, encrypted on the device, and the owner's passphrase for them kept in
//! step on the devices the owner let in (migrations 6 and 11).
//!
//! - Keys (Web search, describe) are encrypted with AES-256-GCM under a key made from the passphrase
//!   (scrypt); the database only ever holds ciphertext.
//! - Every passphrase ever set is a version: its scrypt settings, a known value encrypted with its key (to
//!   tell a right passphrase from a wrong one), the key's fingerprint, who set it and when. One is current.
//! - Each device has its own X25519 key pair: the private half in `HOME/device-key.json` (protected like
//!   the database password), the public half in `devices.public_key`.
//! - The database password alone gets nobody the passphrase. A device gets it only from the owner: typed
//!   there (`set_passphrase`), or asked for (`ask`: a one-time code shown on that device, never in the
//!   database) and the code typed on a device that has it (`approve`). The code's check values prove each
//!   side to the other.
//! - A device that holds the current passphrase shows it (`devices.key_proof`: a keyed check value over its
//!   id and public key that only a holder can make). A change or restore is sealed only to devices that show
//!   the passphrase it replaces, and carries a check value made with that passphrase's key over which
//!   version it replaces: a device follows a new version only when that fits the passphrase it has. So
//!   nobody without the passphrase can hand a device another one.
//! - A device that isn't on the current version neither sends nor reads keys.
//!
//! What someone with only the database password can still do: delete or damage the encrypted keys and
//! these tables, which stops keys syncing until the owner sets things right. Not read a key or the
//! passphrase. Don't add a path that seals the passphrase to a device on the strength of a row.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use face_files::{filejson, paths::utc_text, protect};
use ring::{
    aead, hkdf, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Map, Value, json};
use tokio_postgres::{Client, GenericClient};
use x25519_dalek::{PublicKey, StaticSecret};

/// (named where a device that lets another one in reads the passphrase to hand on: `face_home::passphrase`)
const KEY_FILE: &str = face_home::passphrase::KEY_FILE;
const DEVICE_KEY: &str = "device-key.json";
/// this device's open request: its code and the key made from it
const REQUEST: &str = "key-request.json";
const CHECK: &[u8] = b"face-recognizer passphrase check";
/// scrypt's settings for a new version: (n, r, p)
const SCRYPT: (i32, i32, i32) = (1 << 15, 8, 1);
pub const MIN_PASSPHRASE: usize = 10;
pub const REQUEST_MINUTES: f64 = 15.0;
/// 32 symbols; no I, O, 0 or 1 (read wrongly off a screen)
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// 40 bits, each guess costing the code's scrypt: not in 15 minutes
const CODE_LEN: usize = 8;
const CODE_SCRYPT: (i32, i32, i32) = (1 << 16, 8, 1);

/// Why something about the keys didn't happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    /// not the passphrase, not the code, not this device's to open: what the owner is told
    Wrong(String),
    /// what was asked can't be done as asked: what the owner is told
    Bad(String),
    /// the database or this device's files
    Failed(String),
}

impl Fault {
    pub fn text(&self) -> &str {
        match self {
            Fault::Wrong(t) | Fault::Bad(t) | Fault::Failed(t) => t,
        }
    }

    /// Something the owner is told, not a failure of the device.
    pub fn refused(&self) -> bool {
        !matches!(self, Fault::Failed(_))
    }
}

type Result<T> = std::result::Result<T, Fault>;

fn db(e: tokio_postgres::Error) -> Fault {
    Fault::Failed(crate::server::words(&e))
}

fn failed(e: impl std::fmt::Display) -> Fault {
    Fault::Failed(e.to_string())
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    SystemRandom::new().fill(&mut out).map_err(|_| failed("this device gave no random numbers"))?;
    Ok(out)
}

// ---- primitives -----------------------------------------------------------------------------------------------

/// The key a passphrase (or a code) makes under these scrypt settings.
pub fn derive(passphrase: &str, salt: &[u8], n: i32, r: i32, p: i32) -> Result<Vec<u8>> {
    let bad = || Fault::Wrong("These aren't settings this app makes.".to_string());
    if n < 2 || n & (n - 1) != 0 || r < 1 || p < 1 {
        return Err(bad());
    }
    let params = scrypt::Params::new(n.trailing_zeros() as u8, r as u32, p as u32).map_err(|_| bad())?;
    let mut key = vec![0u8; 32];
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, &mut key).map_err(|_| bad())?;
    Ok(key)
}

fn gcm(key: &[u8]) -> Result<aead::LessSafeKey> {
    let key =
        aead::UnboundKey::new(&aead::AES_256_GCM, key).map_err(|_| Fault::Wrong("That is not a key.".to_string()))?;
    Ok(aead::LessSafeKey::new(key))
}

fn seal_with(key: &[u8], nonce: [u8; 12], name: &[u8], plain: &[u8]) -> Result<Vec<u8>> {
    let mut data = plain.to_vec();
    gcm(key)?
        .seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(name), &mut data)
        .map_err(|_| failed("couldn't encrypt"))?;
    Ok(data)
}

fn open_with(key: &[u8], nonce: &[u8], name: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut data = ciphertext.to_vec();
    let plain = gcm(key).ok()?.open_in_place(nonce, aead::Aad::from(name), &mut data).ok()?.len();
    data.truncate(plain);
    Some(data)
}

/// (nonce, ciphertext) of `plain`; `name` is bound to it, so a ciphertext can't be moved to another name.
pub fn encrypt(key: &[u8], name: &str, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let nonce: [u8; 12] = random()?;
    Ok((nonce.to_vec(), seal_with(key, nonce, name.as_bytes(), plain)?))
}

pub fn decrypt(key: &[u8], name: &str, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    open_with(key, nonce, name.as_bytes(), ciphertext)
        .ok_or_else(|| Fault::Wrong("This key was encrypted with another passphrase.".to_string()))
}

fn mac_of(key: &[u8], data: &[&[u8]]) -> hmac::Tag {
    let mut sum = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, key));
    for part in data {
        sum.update(part);
    }
    sum.sign()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn in_fours(tag: &hmac::Tag) -> String {
    let h = hex(tag.as_ref())[..12].to_uppercase();
    format!("{}-{}-{}", &h[..4], &h[4..8], &h[8..])
}

/// A short one-way fingerprint of a passphrase key: XXXX-XXXX-XXXX.
pub fn fingerprint(key: &[u8]) -> String {
    in_fours(&mac_of(key, &[b"face-recognizer key fingerprint"]))
}

/// The same for the database password, keyed with the database's own id (so it means nothing elsewhere).
pub fn password_fingerprint(password: &str, instance: &str) -> String {
    in_fours(&mac_of(instance.as_bytes(), &[password.as_bytes()]))
}

/// A keyed digest of a secret (for the sync's "did it change"), never the secret itself.
pub fn digest(key: &[u8], name: &str, plain: &[u8]) -> String {
    hex(mac_of(key, &[name.as_bytes(), b"\0", plain]).as_ref())
}

/// HMAC-SHA256 under a key made from `key` for this one purpose, over `parts` (each with its length, so two
/// lists of parts never read the same).
fn mac(key: &[u8], purpose: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let sub = mac_of(key, &[b"face-recognizer ", purpose]);
    let mut sum = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, sub.as_ref()));
    for part in parts {
        sum.update(&(part.len() as u32).to_be_bytes());
        sum.update(part);
    }
    sum.sign().as_ref().to_vec()
}

/// What a device holding `key` writes into `devices.key_proof`: bound to its id and public key.
pub fn device_proof(key: &[u8], device: &str, public: &[u8]) -> Vec<u8> {
    mac(key, b"device proof", &[device.as_bytes(), public])
}

/// What the device making version `vid` writes into its link: made with the key of the version it replaces,
/// and saying which one that was. (Without that, a restore, which gives a new version the key of an earlier
/// one, made the link of whatever once replaced that earlier one fit again.)
fn link(prev_key: &[u8], prev: i32, vid: i32, fingerprint: &str) -> Vec<u8> {
    mac(prev_key, b"version link 2", &[prev.to_string().as_bytes(), vid.to_string().as_bytes(), fingerprint.as_bytes()])
}

/// Two check values are the same (compared without telling where they differ).
fn same(a: Option<&[u8]>, b: &[u8]) -> bool {
    a.is_some_and(|a| a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0)
}

struct Len32;

impl hkdf::KeyType for Len32 {
    fn len(&self) -> usize {
        32
    }
}

fn box_key(shared: &[u8], sender: &[u8], recipient: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    hkdf::Salt::new(hkdf::HKDF_SHA256, &[])
        .extract(shared)
        .expand(&[b"face-recognizer passphrase share", sender, recipient], Len32)
        .and_then(|okm| okm.fill(&mut key))
        .map_err(|_| failed("couldn't make a key"))?;
    Ok(key)
}

fn public_key(bytes: &[u8]) -> Result<PublicKey> {
    let raw: [u8; 32] = bytes.try_into().map_err(|_| Fault::Bad("That is not a device's public key.".to_string()))?;
    Ok(PublicKey::from(raw))
}

fn agree(private: &StaticSecret, theirs: &PublicKey) -> Result<[u8; 32]> {
    let shared = private.diffie_hellman(theirs);
    if !shared.was_contributory() {
        return Err(Fault::Bad("That is not a device's public key.".to_string()));
    }
    Ok(shared.to_bytes())
}

/// `plain` sealed to a device's X25519 public key: a one-time public key, a nonce, the ciphertext.
pub fn seal(recipient: &[u8], plain: &str) -> Result<Vec<u8>> {
    let theirs = public_key(recipient)?;
    let once = StaticSecret::from(random::<32>()?);
    let ours = PublicKey::from(&once);
    let key = box_key(&agree(&once, &theirs)?, ours.as_bytes(), recipient)?;
    let nonce: [u8; 12] = random()?;
    let mut out = ours.as_bytes().to_vec();
    out.extend(nonce);
    out.extend(seal_with(&key, nonce, b"", plain.as_bytes())?);
    Ok(out)
}

fn unseal_with(private: &StaticSecret, sealed: &[u8]) -> Result<String> {
    let another = || Fault::Wrong("This share was sealed to another device.".to_string());
    if sealed.len() < 44 {
        return Err(Fault::Bad("That is not a sealed passphrase.".to_string()));
    }
    let (sender, rest) = sealed.split_at(32);
    let (nonce, ciphertext) = rest.split_at(12);
    let ours = PublicKey::from(private);
    let key = box_key(&agree(private, &public_key(sender)?)?, sender, ours.as_bytes())?;
    let plain = open_with(&key, nonce, b"", ciphertext).ok_or_else(another)?;
    String::from_utf8(plain).map_err(|_| Fault::Bad("That is not a sealed passphrase.".to_string()))
}

fn code_key(code: &str, salt: &[u8]) -> Result<Vec<u8>> {
    derive(code, salt, CODE_SCRYPT.0, CODE_SCRYPT.1, CODE_SCRYPT.2)
}

/// A code as typed, without what isn't a letter or digit of it.
fn norm(code: &str) -> String {
    code.to_uppercase().chars().filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).collect()
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

// ---- the database's versions ----------------------------------------------------------------------------------

/// One passphrase version as the database has it.
#[derive(Debug, Clone)]
pub struct Version {
    pub id: i32,
    salt: Vec<u8>,
    n: i32,
    r: i32,
    p: i32,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    pub fingerprint: String,
    prev: Option<i32>,
    link: Option<Vec<u8>>,
}

const VERSION_COLS: &str = "id, salt, n, r, p, nonce, ciphertext, fingerprint, prev, link";

fn version_of(row: &tokio_postgres::Row) -> Version {
    Version {
        id: row.get(0),
        salt: row.get(1),
        n: row.get(2),
        r: row.get(3),
        p: row.get(4),
        nonce: row.get(5),
        ciphertext: row.get(6),
        fingerprint: row.get(7),
        prev: row.get(8),
        link: row.get(9),
    }
}

/// The current version, if a passphrase was ever set.
pub async fn current(c: &impl GenericClient) -> Result<Option<Version>> {
    let ask = format!("SELECT {VERSION_COLS} FROM passphrase_versions WHERE current");
    Ok(c.query_opt(&ask, &[]).await.map_err(db)?.as_ref().map(version_of))
}

/// The key `passphrase` makes under a version's settings, or that it is the wrong one. The settings come
/// from the database: ones no version of this app would have written are refused, not run.
fn key_for(version: &Version, passphrase: &str) -> Result<Vec<u8>> {
    let (n, r, p) = (version.n, version.r, version.p);
    if !((1 << 14..=1 << 17).contains(&n) && n & (n - 1) == 0 && r == 8 && p == 1) {
        return Err(Fault::Wrong("This passphrase version's settings aren't ones this app makes.".to_string()));
    }
    let key = derive(passphrase, &version.salt, n, r, p)?;
    if decrypt(&key, "check", &version.nonce, &version.ciphertext)? != CHECK {
        return Err(Fault::Wrong(String::new()));
    }
    Ok(key)
}

/// This device's copy of the passphrase and its own key pair, in its private folder.
pub struct Keys {
    home: PathBuf,
    /// this device's id
    device: String,
}

impl Keys {
    pub fn new(home: &Path, device: &str) -> Keys {
        Keys { home: home.to_path_buf(), device: device.to_string() }
    }

    fn read(&self, file: &str) -> Map<String, Value> {
        let read = std::fs::read(self.home.join(file)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        read.and_then(|v| v.as_object().cloned()).unwrap_or_default()
    }

    /// (readable only by this user)
    fn write(&self, file: &str, v: &Value) -> Result<()> {
        protect::write_private(&self.home.join(file), filejson::dumps_lines(v, true, 1).as_bytes()).map_err(failed)
    }

    fn kept(d: &Map<String, Value>, k: &str) -> Option<String> {
        protect::unprotect(d.get(k)?.as_str()?).ok()
    }

    /// This device's passphrase key, if it has one.
    pub fn local_key(&self) -> Option<Vec<u8>> {
        B64.decode(Keys::kept(&self.read(KEY_FILE), "key")?).ok()
    }

    pub fn local_version(&self) -> Option<i32> {
        self.read(KEY_FILE).get("version")?.as_i64().and_then(|v| i32::try_from(v).ok())
    }

    /// The passphrase this device has ("" if none).
    pub fn saved_passphrase(&self) -> String {
        Keys::kept(&self.read(KEY_FILE), "passphrase").unwrap_or_default()
    }

    /// {version id: fingerprint} of every version this device held itself (the ones it may restore).
    fn known(&self) -> Map<String, Value> {
        self.read(KEY_FILE).get("known").and_then(Value::as_object).cloned().unwrap_or_default()
    }

    fn keep(&self, passphrase: &str, key: &[u8], version: i32) -> Result<()> {
        let mut known = self.known();
        known.insert(version.to_string(), json!(fingerprint(key)));
        let protect = |text: &str| protect::protect(text).map_err(failed);
        self.write(KEY_FILE, &json!({"key": protect(&B64.encode(key))?, "passphrase": protect(passphrase)?, "version": version, "known": known}))
    }

    pub fn forget_local(&self) {
        let _ = std::fs::remove_file(self.home.join(KEY_FILE));
    }

    /// This device's own private key, made the first time it is asked for.
    fn device_private(&self) -> Result<StaticSecret> {
        let kept = Keys::kept(&self.read(DEVICE_KEY), "private").and_then(|b| B64.decode(b).ok());
        if let Some(raw) = kept.and_then(|b| <[u8; 32]>::try_from(b).ok()) {
            return Ok(StaticSecret::from(raw));
        }
        let raw: [u8; 32] = random()?;
        self.write(DEVICE_KEY, &json!({"private": protect::protect(&B64.encode(raw)).map_err(failed)?}))?;
        Ok(StaticSecret::from(raw))
    }

    pub fn device_public(&self) -> Result<[u8; 32]> {
        Ok(PublicKey::from(&self.device_private()?).to_bytes())
    }

    /// A passphrase sealed to this device, opened.
    pub fn unseal(&self, sealed: &[u8]) -> Result<String> {
        unseal_with(&self.device_private()?, sealed)
    }

    /// This device holds the current version's key.
    pub fn on_current(&self, cur: Option<&Version>) -> bool {
        match (cur, self.local_key()) {
            (Some(cur), Some(key)) => {
                self.local_version() == Some(cur.id)
                    && same(Some(fingerprint(&key).as_bytes()), cur.fingerprint.as_bytes())
            }
            _ => false,
        }
    }

    /// This device's share of a version, opened; None when it has none.
    async fn version_passphrase(&self, c: &impl GenericClient, vid: i32) -> Result<Option<String>> {
        let ask = "SELECT sealed FROM passphrase_shares WHERE version = $1 AND device = $2::text::uuid";
        match c.query_opt(ask, &[&vid, &self.device]).await.map_err(db)? {
            Some(row) => Ok(Some(self.unseal(row.get::<_, &[u8]>(0))?)),
            None => Ok(None),
        }
    }

    /// A new version (not yet current). `prev`: the version it replaces with that one's key, for its link.
    async fn new_version(
        &self,
        c: &impl GenericClient,
        passphrase: &str,
        note: &str,
        prev: Option<(i32, &[u8])>,
    ) -> Result<(i32, Vec<u8>)> {
        let salt: [u8; 16] = random()?;
        let key = derive(passphrase, &salt, SCRYPT.0, SCRYPT.1, SCRYPT.2)?;
        let (nonce, check) = encrypt(&key, "check", CHECK)?;
        let add =
            "INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext, fingerprint, note, prev)
                   VALUES ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id";
        let print = fingerprint(&key);
        let row = c
            .query_one(
                add,
                &[
                    &self.device,
                    &salt.as_slice(),
                    &SCRYPT.0,
                    &SCRYPT.1,
                    &SCRYPT.2,
                    &nonce,
                    &check,
                    &print,
                    &note,
                    &prev.map(|p| p.0),
                ],
            )
            .await
            .map_err(db)?;
        let vid: i32 = row.get(0);
        if let Some((prev, prev_key)) = prev {
            c.execute(
                "UPDATE passphrase_versions SET link = $1 WHERE id = $2",
                &[&link(prev_key, prev, vid, &print), &vid],
            )
            .await
            .map_err(db)?;
        }
        Ok((vid, key))
    }

    /// [(device id, public key)] of the devices that show they hold `key`.
    async fn holders(c: &impl GenericClient, key: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
        let ask = "SELECT id::text, public_key, key_proof FROM devices WHERE public_key IS NOT NULL AND key_proof IS NOT NULL";
        let rows = c.query(ask, &[]).await.map_err(db)?;
        Ok((rows.iter())
            .map(|r| (r.get::<_, String>(0), r.get::<_, Vec<u8>>(1), r.get::<_, Vec<u8>>(2)))
            .filter(|(device, public, proof)| same(Some(proof), &device_proof(key, device, public)))
            .map(|(device, public, _)| (device, public))
            .collect())
    }

    async fn put_share(
        c: &impl GenericClient,
        vid: i32,
        device: &str,
        public: &[u8],
        passphrase: &str,
    ) -> Result<bool> {
        let sealed = match seal(public, passphrase) {
            Ok(s) => s,
            Err(e) if e.refused() => return Ok(false), // not a public key
            Err(e) => return Err(e),
        };
        let put = "INSERT INTO passphrase_shares (version, device, sealed) VALUES ($1, $2::text::uuid, $3)
                   ON CONFLICT (version, device) DO UPDATE SET sealed = excluded.sealed";
        c.execute(put, &[&vid, &device, &sealed]).await.map_err(db)?;
        Ok(true)
    }

    /// Seal a version's passphrase to this device, and to every device that shows it holds the key `held`
    /// (the current passphrase's, or the one the version replaces) and hasn't got the version yet. Never to
    /// a device that only has a row in `devices`: the database password alone must not be enough to be sent
    /// the passphrase. How many others.
    async fn share_all(
        &self,
        c: &impl GenericClient,
        vid: i32,
        passphrase: &str,
        held: Option<&[u8]>,
    ) -> Result<usize> {
        let rows =
            c.query("SELECT device::text FROM passphrase_shares WHERE version = $1", &[&vid]).await.map_err(db)?;
        let have: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        if !have.contains(&self.device) {
            Keys::put_share(c, vid, &self.device, &self.device_public()?, passphrase).await?;
        }
        let mut n = 0;
        if let Some(held) = held {
            for (device, public) in Keys::holders(c, held).await? {
                if device != self.device && !have.contains(&device) {
                    n += usize::from(Keys::put_share(c, vid, &device, &public, passphrase).await?);
                }
            }
        }
        Ok(n)
    }

    /// This device's public key, fingerprints and proof in the devices table (each sync step; cheap).
    /// `password`: the database password's fingerprint (`password_fingerprint`), when it is to be shown.
    pub async fn register(&self, c: &impl GenericClient, password: Option<&str>) -> Result<()> {
        let (key, public) = (self.local_key(), self.device_public()?);
        let said = "UPDATE devices SET public_key = $1, key_version = $2, key_fingerprint = $3, key_proof = $4,
                    password_fingerprint = coalesce($5, password_fingerprint) WHERE id = $6::text::uuid";
        let (print, proof) =
            (key.as_deref().map(fingerprint), key.as_deref().map(|k| device_proof(k, &self.device, &public)));
        c.execute(said, &[&public.as_slice(), &self.local_version(), &print, &proof, &password, &self.device])
            .await
            .map_err(db)?;
        Ok(())
    }

    /// Take the current version when it replaced the one this device has and was made by a device holding
    /// that one (its link fits this device's key).
    async fn follow(&self, c: &impl GenericClient, cur: &Version) -> Result<bool> {
        let (Some(key), Some(mine)) = (self.local_key(), self.local_version()) else { return Ok(false) };
        if cur.prev != Some(mine) || !same(cur.link.as_deref(), &link(&key, mine, cur.id, &cur.fingerprint)) {
            return Ok(false);
        }
        // (not sealed to this device, not a share at all, a wrong passphrase: not followed)
        let opened = match self.version_passphrase(c, cur.id).await {
            Ok(Some(p)) => key_for(cur, &p).map(|k| (p, k)),
            Ok(None) => return Ok(false),
            Err(e) => Err(e),
        };
        match opened {
            Ok((passphrase, new)) if fingerprint(&new) == cur.fingerprint => {
                self.keep(&passphrase, &new, cur.id)?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) if e.refused() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Once, on a device coming from a version of the app that kept no list of its own versions: note the
    /// versions it holds shares of as its own (it received them under the old rules), so it can still
    /// restore them.
    async fn learn(&self, c: &impl GenericClient) -> Result<()> {
        let mut kept = self.read(KEY_FILE);
        let Some(key) = self.local_key().filter(|_| !kept.contains_key("known")) else { return Ok(()) };
        let ask = "SELECT v.id, v.salt, v.n, v.r, v.p, v.nonce, v.ciphertext, v.fingerprint, NULL::int, NULL::bytea, s.sealed
                   FROM passphrase_shares s JOIN passphrase_versions v ON v.id = s.version WHERE s.device = $1::text::uuid";
        let mut known = Map::new();
        for row in c.query(ask, &[&self.device]).await.map_err(db)? {
            let version = version_of(&row);
            let fits = self
                .unseal(row.get::<_, &[u8]>(10))
                .and_then(|p| key_for(&version, &p))
                .is_ok_and(|k| fingerprint(&k) == version.fingerprint);
            if fits {
                known.insert(version.id.to_string(), json!(version.fingerprint));
            }
        }
        if let Some(version) = kept.get("version").and_then(Value::as_i64) {
            known.insert(version.to_string(), json!(fingerprint(&key)));
        }
        kept.insert("known".to_string(), Value::Object(known));
        self.write(KEY_FILE, &Value::Object(kept))
    }

    /// Take the current passphrase if this device doesn't have it yet and may: a change made by a device
    /// that held the one it has, or the answer to its own request. Then seal the current one to any device
    /// that shows it holds it and has no share yet. (has the current one, just took it)
    pub async fn catch_up(&self, c: &impl GenericClient) -> Result<(bool, bool)> {
        let Some(cur) = current(c).await? else { return Ok((false, false)) };
        self.learn(c).await?;
        let mut took = false;
        if !self.on_current(Some(&cur)) {
            took = self.follow(c, &cur).await? || self.take_answer(c, &cur).await?;
            if !took {
                return Ok((false, false));
            }
        }
        if self.home.join(REQUEST).exists() {
            self.drop_request(c).await?;
        }
        self.share_all(c, cur.id, &self.saved_passphrase(), self.local_key().as_deref()).await?;
        Ok((true, took))
    }

    /// Take a passphrase that the device which let this one in handed over (`face_db::join`), when it is the
    /// current one: what typing it here would do (`set_passphrase`'s "entered"), and nothing else. One that
    /// isn't the current one, or one handed over where none was ever set, changes nothing: what another
    /// device hands over never sets or changes the passphrase. Whether this device has the current one now
    /// (`catch_up`, which follows in the same step, then seals it on and ends an open request).
    pub async fn enter(&self, c: &impl GenericClient, passphrase: &str) -> Result<bool> {
        let Some(cur) = current(c).await? else { return Ok(false) };
        if self.on_current(Some(&cur)) {
            return Ok(true);
        }
        match key_for(&cur, passphrase) {
            Ok(key) => {
                self.keep(passphrase, &key, cur.id)?;
                Ok(true)
            }
            Err(e) if e.refused() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Re-encrypt every key from `old` to `new` and make a version current (inside the caller's transaction).
    async fn make_current(c: &impl GenericClient, vid: i32, new: &[u8], old: Option<&[u8]>) -> Result<()> {
        if let Some(old) = old {
            for row in c.query("SELECT name, nonce, ciphertext FROM secrets", &[]).await.map_err(db)? {
                let name: String = row.get(0);
                let plain = decrypt(old, &name, row.get(1), row.get(2))?;
                let (nonce, ciphertext) = encrypt(new, &name, &plain)?;
                c.execute(
                    "UPDATE secrets SET nonce = $1, ciphertext = $2 WHERE name = $3",
                    &[&nonce, &ciphertext, &name],
                )
                .await
                .map_err(db)?;
            }
        }
        c.execute("UPDATE passphrase_versions SET current = false WHERE current AND id <> $1", &[&vid])
            .await
            .map_err(db)?;
        c.execute("UPDATE passphrase_versions SET current = true WHERE id = $1", &[&vid]).await.map_err(db)?;
        Ok(())
    }

    /// Set, enter or change the passphrase from this device.
    /// - None set yet: it becomes version 1, on this device (the others: type it there, or ask and approve).
    /// - The current one: this device takes it (the way in for a new device: "entered").
    /// - Another one: a change, allowed only on a device that has the current passphrase; every key is
    ///   re-encrypted and the new passphrase sealed to every device that shows the current one. `again`
    ///   must match (typed twice).
    ///
    /// {ok, what: set | entered | changed, version, fingerprint}
    pub async fn set_passphrase(&self, client: &mut Client, passphrase: &str, again: Option<&str>) -> Result<Value> {
        if again.is_some_and(|a| a != passphrase) {
            return Err(Fault::Bad("The two passphrases aren't the same.".to_string()));
        }
        let short = || Fault::Bad(format!("Use a passphrase of at least {MIN_PASSPHRASE} characters."));
        let long_enough = passphrase.chars().count() >= MIN_PASSPHRASE;
        self.register(client, None).await?;
        let tx = client.transaction().await.map_err(db)?;
        tx.batch_execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE").await.map_err(db)?;
        let (what, vid, key) = match current(&tx).await? {
            Some(cur) => match key_for(&cur, passphrase) {
                Ok(key) => {
                    self.keep(passphrase, &key, cur.id)?;
                    self.share_all(&tx, cur.id, passphrase, None).await?;
                    ("entered", cur.id, key)
                }
                Err(Fault::Wrong(_)) => {
                    let Some(old) = self.local_key().filter(|_| self.on_current(Some(&cur))) else {
                        return Err(Fault::Wrong(
                            "That isn't the current passphrase. Type the one your other devices have, or get it from one of them with a code.".to_string(),
                        ));
                    };
                    if !long_enough {
                        return Err(short());
                    }
                    let (vid, key) = self.new_version(&tx, passphrase, "changed", Some((cur.id, &old))).await?;
                    Keys::make_current(&tx, vid, &key, Some(&old)).await?;
                    self.share_all(&tx, vid, passphrase, Some(&old)).await?;
                    self.keep(passphrase, &key, vid)?;
                    ("changed", vid, key)
                }
                Err(e) => return Err(e),
            },
            None => {
                if !long_enough {
                    return Err(short());
                }
                let (vid, key) = self.new_version(&tx, passphrase, "set", None).await?;
                Keys::make_current(&tx, vid, &key, None).await?;
                self.share_all(&tx, vid, passphrase, None).await?;
                self.keep(passphrase, &key, vid)?;
                ("set", vid, key)
            }
        };
        tx.commit().await.map_err(db)?;
        self.register(client, None).await?;
        self.drop_request(client).await?;
        Ok(json!({"ok": true, "what": what, "version": vid, "fingerprint": fingerprint(&key)}))
    }

    /// Make an earlier version current again (undo a wrong change): every key is re-encrypted to it and
    /// every device that shows the current passphrase takes it. Only on a device that has the current
    /// passphrase and held that earlier one itself: a version this device only finds in the database could
    /// be anyone's.
    pub async fn restore(&self, client: &mut Client, vid: i32) -> Result<Value> {
        self.register(client, None).await?;
        let tx = client.transaction().await.map_err(db)?;
        tx.batch_execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE").await.map_err(db)?;
        let cur = current(&tx).await?;
        let ask = format!("SELECT {VERSION_COLS} FROM passphrase_versions WHERE id = $1");
        let Some(target) = tx.query_opt(&ask, &[&vid]).await.map_err(db)?.as_ref().map(version_of) else {
            return Err(Fault::Bad("No such passphrase version.".to_string()));
        };
        if cur.as_ref().is_some_and(|c| c.id == target.id) {
            return Err(Fault::Bad("That's the current passphrase already.".to_string()));
        }
        if cur.is_some() && !self.on_current(cur.as_ref()) {
            return Err(Fault::Wrong(
                "This device doesn't have the current passphrase; restore from a device that has.".to_string(),
            ));
        }
        if self.known().get(&target.id.to_string()).and_then(Value::as_str) != Some(target.fingerprint.as_str()) {
            return Err(Fault::Wrong(
                "This device never had that passphrase; restore it from a device that had.".to_string(),
            ));
        }
        let Some(passphrase) = self.version_passphrase(&tx, target.id).await? else {
            return Err(Fault::Wrong(
                "This device hasn't received that passphrase; restore it from a device that has.".to_string(),
            ));
        };
        let old = cur.as_ref().and_then(|_| self.local_key());
        let new = key_for(&target, &passphrase)?;
        if fingerprint(&new) != target.fingerprint {
            return Err(Fault::Wrong("That passphrase version was changed in the database.".to_string()));
        }
        // a new version with the earlier passphrase, so the history shows the restore
        let add = "INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext, fingerprint, note, prev)
                   SELECT $1::text::uuid, salt, n, r, p, nonce, ciphertext, fingerprint, $2, $3::int FROM passphrase_versions
                   WHERE id = $4 RETURNING id";
        let note = format!("restored version {}", target.id);
        let again: i32 = tx
            .query_one(add, &[&self.device, &note, &cur.as_ref().map(|c| c.id), &target.id])
            .await
            .map_err(db)?
            .get(0);
        if let (Some(old), Some(cur)) = (old.as_deref(), cur.as_ref()) {
            let fits = link(old, cur.id, again, &target.fingerprint);
            tx.execute("UPDATE passphrase_versions SET link = $1 WHERE id = $2", &[&fits, &again]).await.map_err(db)?;
        }
        Keys::make_current(&tx, again, &new, old.as_deref()).await?;
        self.share_all(&tx, again, &passphrase, old.as_deref()).await?;
        self.keep(&passphrase, &new, again)?;
        tx.commit().await.map_err(db)?;
        self.register(client, None).await?;
        Ok(json!({"ok": true, "what": "restored", "version": again, "fingerprint": fingerprint(&new)}))
    }

    // ---- a device without the passphrase asks; the owner approves it on one that has it ----------------------

    /// This device's open request: {code: "XXXX-XXXX", until: UTC time} while it lasts.
    pub fn request(&self) -> Option<Value> {
        let kept = self.read(REQUEST);
        let until = kept.get("until")?.as_f64()?;
        if until <= now_secs() {
            let _ = std::fs::remove_file(self.home.join(REQUEST));
            return None;
        }
        let code = Keys::kept(&kept, "code")?;
        let until = utc_text(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(until.max(0.0)));
        Some(json!({"code": format!("{}-{}", code.get(..4)?, code.get(4..)?), "until": until}))
    }

    fn request_key(&self) -> Option<Vec<u8>> {
        let kept = self.read(REQUEST);
        if kept.get("until")?.as_f64()? <= now_secs() {
            return None;
        }
        B64.decode(Keys::kept(&kept, "k")?).ok()
    }

    async fn drop_request(&self, c: &impl GenericClient) -> Result<()> {
        let _ = std::fs::remove_file(self.home.join(REQUEST));
        c.execute("DELETE FROM key_requests WHERE device = $1::text::uuid", &[&self.device]).await.map_err(db)?;
        Ok(())
    }

    /// This device asks for the passphrase: a new one-time code, shown only here, and a request in the
    /// database with a check value made from the code over this device's id and public key.
    pub async fn ask(&self, c: &impl GenericClient) -> Result<Value> {
        self.register(c, None).await?;
        let Some(cur) = current(c).await? else {
            return Err(Fault::Bad("No passphrase has been set yet: set one on this device.".to_string()));
        };
        if self.on_current(Some(&cur)) {
            return Err(Fault::Bad("This device has the passphrase already.".to_string()));
        }
        let public = self.device_public()?;
        // (32 symbols: every byte's low five bits pick one, each as likely as the others)
        let code: String = random::<CODE_LEN>()?.iter().map(|b| CODE_ALPHABET[(b & 31) as usize] as char).collect();
        let salt: [u8; 16] = random()?;
        let k = code_key(&code, &salt)?;
        let ask = "INSERT INTO key_requests (device, public_key, salt, mac) VALUES ($1::text::uuid, $2, $3, $4)
                   ON CONFLICT (device) DO UPDATE SET public_key = excluded.public_key, salt = excluded.salt,
                   mac = excluded.mac, asked_at = now(), version = NULL, sealed = NULL, answer_mac = NULL,
                   answered_by = NULL, answered_at = NULL";
        let check = mac(&k, b"key request", &[self.device.as_bytes(), &public]);
        c.execute(ask, &[&self.device, &public.as_slice(), &salt.as_slice(), &check]).await.map_err(db)?;
        let protect = |text: &str| protect::protect(text).map_err(failed);
        self.write(REQUEST, &json!({"code": protect(&code)?, "k": protect(&B64.encode(&k))?, "until": now_secs() + REQUEST_MINUTES * 60.0}))?;
        self.request().ok_or_else(|| failed("the request couldn't be kept"))
    }

    /// The other devices asking now: [{id, name, asked}].
    pub async fn waiting(&self, c: &impl GenericClient) -> Result<Vec<Value>> {
        let ask = "SELECT r.device::text, coalesce(nullif(btrim(d.alias), ''), d.name),
                          to_char(r.asked_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')
                   FROM key_requests r JOIN devices d ON d.id = r.device
                   WHERE r.sealed IS NULL AND r.asked_at > now() - make_interval(mins => $1::int) AND r.device <> $2::text::uuid
                   ORDER BY r.asked_at";
        let rows = c.query(ask, &[&(REQUEST_MINUTES as i32), &self.device]).await.map_err(db)?;
        Ok(rows
            .iter()
            .map(
                |r| json!({"id": r.get::<_, String>(0), "name": r.get::<_, String>(1), "asked": r.get::<_, String>(2)}),
            )
            .collect())
    }

    /// The owner typed, on this device, the code a waiting device shows: check the code against that
    /// device's request (so the public key in it is the one of the device showing the code), then answer
    /// with the passphrase sealed to that key and a check value made from the code (so the asking device
    /// knows the answer came from where its code was typed). {ok, what: "approved", given: its name}
    pub async fn approve(&self, client: &mut Client, code: &str, device: Option<&str>) -> Result<Value> {
        let code = norm(code);
        if code.len() != CODE_LEN {
            return Err(Fault::Bad(format!("The code has {CODE_LEN} letters and digits.")));
        }
        let tx = client.transaction().await.map_err(db)?;
        tx.batch_execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE").await.map_err(db)?;
        let cur = current(&tx).await?;
        let Some(cur) = cur.filter(|c| self.on_current(Some(c))) else {
            return Err(Fault::Wrong("This device doesn't have the current passphrase itself.".to_string()));
        };
        let ask = "SELECT r.device::text, coalesce(nullif(btrim(d.alias), ''), d.name), r.public_key, r.salt, r.mac
                   FROM key_requests r JOIN devices d ON d.id = r.device
                   WHERE r.sealed IS NULL AND r.asked_at > now() - make_interval(mins => $1::int) AND r.device <> $2::text::uuid
                   ORDER BY r.asked_at DESC LIMIT 8";
        let rows = tx.query(ask, &[&(REQUEST_MINUTES as i32), &self.device]).await.map_err(db)?;
        let asking: Vec<_> =
            rows.iter().filter(|r| device.is_none_or(|d| d.is_empty() || d == r.get::<_, String>(0))).collect();
        if asking.is_empty() {
            return Err(Fault::Bad(format!(
                "No device is waiting (a code lasts {REQUEST_MINUTES} minutes): ask again on that device."
            )));
        }
        for row in asking {
            let (asker, name): (String, String) = (row.get(0), row.get(1));
            let (public, salt, check): (Vec<u8>, Vec<u8>, Vec<u8>) = (row.get(2), row.get(3), row.get(4));
            let k = code_key(&code, &salt)?;
            if !same(Some(&check), &mac(&k, b"key request", &[asker.as_bytes(), &public])) {
                continue;
            }
            let Ok(sealed) = seal(&public, &self.saved_passphrase()) else { continue };
            let answer = mac(
                &k,
                b"key answer",
                &[cur.id.to_string().as_bytes(), cur.fingerprint.as_bytes(), asker.as_bytes(), &sealed],
            );
            let said =
                "UPDATE key_requests SET version = $1, sealed = $2, answer_mac = $3, answered_by = $4::text::uuid,
                        answered_at = now() WHERE device = $5::text::uuid";
            tx.execute(said, &[&cur.id, &sealed, &answer, &self.device, &asker]).await.map_err(db)?;
            let put = "INSERT INTO passphrase_shares (version, device, sealed) VALUES ($1, $2::text::uuid, $3)
                       ON CONFLICT (version, device) DO UPDATE SET sealed = excluded.sealed";
            tx.execute(put, &[&cur.id, &asker, &sealed]).await.map_err(db)?;
            tx.commit().await.map_err(db)?;
            return Ok(json!({"ok": true, "what": "approved", "given": name}));
        }
        Err(Fault::Wrong("That isn't the code the waiting device shows.".to_string()))
    }

    /// Take the answer to this device's request when it carries the check value only someone who was given
    /// this device's code can make.
    async fn take_answer(&self, c: &impl GenericClient, cur: &Version) -> Result<bool> {
        let Some(k) = self.request_key() else { return Ok(false) };
        let ask = "SELECT version, sealed, answer_mac FROM key_requests WHERE device = $1::text::uuid";
        let Some(row) = c.query_opt(ask, &[&self.device]).await.map_err(db)? else { return Ok(false) };
        let (version, sealed, check): (Option<i32>, Option<Vec<u8>>, Option<Vec<u8>>) =
            (row.get(0), row.get(1), row.get(2));
        let (Some(sealed), true) = (sealed, version == Some(cur.id)) else { return Ok(false) };
        let fits = mac(
            &k,
            b"key answer",
            &[cur.id.to_string().as_bytes(), cur.fingerprint.as_bytes(), self.device.as_bytes(), &sealed],
        );
        if !same(check.as_deref(), &fits) {
            return Ok(false);
        }
        let opened = self.unseal(&sealed).and_then(|p| key_for(cur, &p).map(|k| (p, k)));
        match opened {
            Ok((passphrase, key)) if fingerprint(&key) == cur.fingerprint => {
                self.keep(&passphrase, &key, cur.id)?;
                self.drop_request(c).await?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) if e.refused() => Ok(false),
            Err(e) => Err(e),
        }
    }

    // ---- for Settings -------------------------------------------------------------------------------------------

    /// For Settings: the versions (newest first), each device's state against the current one, whether this
    /// device has it (here), whether it is asking (asking), and who is asking (waiting; only told to a
    /// device that can approve). A device counts as current when it shows it holds the passphrase, which
    /// this device can check only when it has it too; otherwise by what the device says of itself.
    pub async fn overview(&self, c: &impl GenericClient) -> Result<Value> {
        let cur = current(c).await?;
        let here = self.on_current(cur.as_ref());
        let key = self.local_key().filter(|_| here);
        let known = self.known();
        let ask = "SELECT v.id, to_char(v.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
                          coalesce(nullif(btrim(d.alias), ''), d.name), v.fingerprint, v.current, v.note
                   FROM passphrase_versions v LEFT JOIN devices d ON d.id = v.created_by ORDER BY v.id DESC";
        let versions: Vec<Value> = (c.query(ask, &[]).await.map_err(db)?.iter())
            .map(|r| {
                let (id, print): (i32, String) = (r.get(0), r.get(3));
                let held = known.get(&id.to_string()).and_then(Value::as_str) == Some(print.as_str());
                json!({"id": id, "when": r.get::<_, String>(1), "by": r.get::<_, Option<String>>(2).unwrap_or_default(), "fingerprint": print,
                       "current": r.get::<_, bool>(4), "note": r.get::<_, String>(5), "known": held})
            })
            .collect();
        let ask = "SELECT id::text, coalesce(nullif(btrim(alias), ''), name), key_version, key_fingerprint, password_fingerprint,
                          public_key, key_proof
                   FROM devices ORDER BY 2";
        let devices: Vec<Value> = (c.query(ask, &[]).await.map_err(db)?.iter())
            .map(|r| {
                let (id, name): (String, String) = (r.get(0), r.get(1));
                let (version, print): (Option<i32>, Option<String>) = (r.get(2), r.get(3));
                let (public, proof): (Option<Vec<u8>>, Option<Vec<u8>>) = (r.get(5), r.get(6));
                let says = cur.as_ref().is_some_and(|c| version == Some(c.id) && print.as_deref() == Some(c.fingerprint.as_str()));
                let proved = |key: &[u8]| public.as_ref().is_some_and(|p| same(proof.as_deref(), &device_proof(key, &id, p)));
                let shown = says && key.as_deref().is_none_or(proved);
                json!({"name": name, "fingerprint": print.unwrap_or_default(), "current": shown, "unproved": says && !shown,
                       "password": r.get::<_, Option<String>>(4).unwrap_or_default(), "me": id == self.device})
            })
            .collect();
        let waiting = if here { self.waiting(c).await? } else { Vec::new() };
        Ok(
            json!({"current": cur.as_ref().map(|c| json!({"id": c.id, "fingerprint": c.fingerprint})), "versions": versions, "devices": devices,
                  "here": here, "asking": self.request().is_some(), "waiting": waiting}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    fn key() -> Vec<u8> {
        (0u8..32).collect()
    }

    fn other() -> Vec<u8> {
        (100u8..132).collect()
    }

    /// (every expected value here is fixed: every device must come to the same for these made-up keys)
    #[test]
    fn a_passphrase_makes_the_key_the_other_devices_make() {
        let salt: Vec<u8> = (0u8..16).collect();
        assert_eq!(
            hex(&derive("a made-up passphrase", &salt, 1 << 15, 8, 1).unwrap()),
            "24f3c4089c79cb4c917e6e1c281c4d96f62d836016d9eaff53183348c69b66ab"
        );
        assert_eq!(
            hex(&code_key("ABCD2345", &salt).unwrap()),
            "befc2449819e65c4d644b62b7197bdde4f81dddf8997e3b5492785eb2c34e034"
        );
        for (n, r, p) in [(0, 8, 1), (1000, 8, 1), (1 << 15, 0, 1), (-4, 8, 1)] {
            assert!(derive("x", &salt, n, r, p).is_err(), "{n} {r} {p}");
        }
    }

    #[test]
    fn fingerprints_and_check_values_are_the_other_devices() {
        assert_eq!(fingerprint(&key()), "DECA-C1B3-A4D5");
        assert_eq!(
            password_fingerprint("a made-up password", "0b0e6b0a-7d1c-4c57-9a55-0123456789ab"),
            "F8B6-C709-E230"
        );
        assert_eq!(password_fingerprint("", "x"), "F27E-6527-D6B8");
        assert_eq!(
            digest(&key(), "google_vision", "a made-up key \u{e9}".as_bytes()),
            "25a83c00d27e40b43007c266a31ac0516a1505dce54911e6165c7b460ede8ff0"
        );
        // (a number or nothing among the parts goes in as its fixed text)
        assert_eq!(
            hex(&mac(&key(), b"key request", &[b"a device", &[1, 2, 3], b"7", b"None"])),
            "51c4823cc6563daeb45e998eecb44cc2f9a93eb5e8a56253bbf9c590e5f5a2f0"
        );
        assert_eq!(
            hex(&device_proof(&key(), "11111111-1111-1111-1111-111111111111", &other())),
            "456242ed5e12e2e09575ea72310d300aecccc151f717a238daca2def3d71d9d1"
        );
        assert_eq!(
            hex(&link(&key(), 3, 4, "ABCD-EF01-2345")),
            "97850146e897959e57bd7c48160fb365f874b25161dbec198d9bc7897b1e5612"
        );
        assert!(
            same(Some(&[1, 2]), &[1, 2])
                && !same(Some(&[1, 2]), &[1, 3])
                && !same(Some(&[1]), &[1, 2])
                && !same(None, &[])
        );
    }

    #[test]
    fn a_key_is_encrypted_as_the_other_devices_encrypt_it_and_only_under_its_name() {
        let nonce: [u8; 12] = std::array::from_fn(|i| i as u8);
        let theirs = unhex("376eb772abc5b67ef535b74818833aec71e6564beb34dec0998134d4ae");
        assert_eq!(seal_with(&key(), nonce, b"check", "plain text \u{e9}".as_bytes()).unwrap(), theirs);
        assert_eq!(decrypt(&key(), "check", &nonce, &theirs).unwrap(), "plain text \u{e9}".as_bytes());
        let wrong = Fault::Wrong("This key was encrypted with another passphrase.".to_string());
        assert_eq!(decrypt(&key(), "another name", &nonce, &theirs), Err(wrong.clone()));
        assert_eq!(decrypt(&other(), "check", &nonce, &theirs), Err(wrong.clone()));
        assert_eq!(decrypt(&key(), "check", &nonce[..11], &theirs), Err(wrong));
        let (nonce, ciphertext) = encrypt(&key(), "a name", b"a secret").unwrap();
        assert_eq!((nonce.len(), ciphertext.len()), (12, 8 + 16));
        assert_eq!(decrypt(&key(), "a name", &nonce, &ciphertext).unwrap(), b"a secret");
        assert_ne!(encrypt(&key(), "a name", b"a secret").unwrap().1, ciphertext, "never the same nonce twice");
    }

    #[test]
    fn a_passphrase_sealed_by_another_device_is_opened_by_the_one_it_was_sealed_to() {
        let private = StaticSecret::from(<[u8; 32]>::try_from(other()).unwrap());
        let public = PublicKey::from(&private).to_bytes();
        assert_eq!(hex(&public), "7d9c24316539825c1896e57f28197746793ce60cbee3ad47da9d07b85fa55e2a");
        // sealed by another device to this key
        let theirs = unhex(
            "6418f5f4dd7eb4c635fa0d21605ec5137aa0328c3e5ac2d075ea5a572aac1152a1e7855b9b4052484a00badc2906bb7e45dcadf9888b0ae18ab63785d9e67503051e78b5e8e0ed883159913315",
        );
        assert_eq!(unseal_with(&private, &theirs).unwrap(), "the passphrase \u{e9}");
        // sealed here
        let ours = seal(&public, "another passphrase").unwrap();
        assert_eq!(ours.len(), 32 + 12 + 18 + 16);
        assert_eq!(unseal_with(&private, &ours).unwrap(), "another passphrase");
        let someone = StaticSecret::from([7u8; 32]);
        assert_eq!(
            unseal_with(&someone, &ours),
            Err(Fault::Wrong("This share was sealed to another device.".to_string()))
        );
        assert!(unseal_with(&private, &ours[..40]).is_err_and(|e| e.refused()));
        // what is not a public key gets nothing sealed to it
        assert!(seal(&[1, 2, 3], "x").is_err_and(|e| e.refused()) && seal(&[0u8; 32], "x").is_err_and(|e| e.refused()));
    }

    #[test]
    fn a_devices_copy_of_the_passphrase_is_kept_protected_and_read_back() {
        let home = std::env::temp_dir().join(format!("face-sync-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let keys = Keys::new(&home, "11111111-1111-1111-1111-111111111111");
        assert_eq!((keys.local_key(), keys.local_version(), keys.saved_passphrase()), (None, None, String::new()));
        keys.keep("a made-up passphrase", &key(), 3).unwrap();
        keys.keep("another made-up one", &other(), 4).unwrap();
        assert_eq!(
            (keys.local_key(), keys.local_version(), keys.saved_passphrase()),
            (Some(other()), Some(4), "another made-up one".to_string())
        );
        assert_eq!(Value::Object(keys.known()), json!({"3": fingerprint(&key()), "4": fingerprint(&other())}));
        let text = std::fs::read_to_string(home.join(KEY_FILE)).unwrap();
        assert!(!text.contains("made-up") && text.starts_with("{\n \"key\": \""), "{}", &text[..20]);
        // its own key pair is made once
        let public = keys.device_public().unwrap();
        assert_eq!(keys.device_public().unwrap(), public);
        assert_eq!(keys.unseal(&seal(&public, "sealed to this device").unwrap()).unwrap(), "sealed to this device");
        // a request that has run out is none
        keys.write(REQUEST, &json!({"code": protect::protect("ABCD2345").unwrap(), "k": protect::protect("AAAA").unwrap(), "until": now_secs() + 60.0})).unwrap();
        assert_eq!(keys.request().unwrap()["code"], "ABCD-2345");
        assert!(keys.request_key().is_some());
        keys.write(
            REQUEST,
            &json!({"code": protect::protect("ABCD2345").unwrap(), "k": "", "until": now_secs() - 1.0}),
        )
        .unwrap();
        assert!(keys.request().is_none() && keys.request_key().is_none() && !home.join(REQUEST).exists());
        assert_eq!(norm(" abcd-2345 \n"), "ABCD2345");
        keys.forget_local();
        assert_eq!(keys.local_key(), None);
        let _ = std::fs::remove_dir_all(home);
    }
}
