"""Keys kept in the shared database, encrypted on the device, and the owner's passphrase for them kept in step on
every device (db/006).

- Keys (Web search, describe) are encrypted with AES-256-GCM under a key made from the passphrase (scrypt); the
  database only ever holds ciphertext.
- Every passphrase ever set is a version (passphrase_versions): its scrypt settings, a known value encrypted with
  its key (to tell a right passphrase from a wrong one), the key's fingerprint, who set it and when. One is current.
- Each device has its own X25519 key pair: the private half in HOME/device-key.json (protected like the database
  password), the public half in devices.public_key. Each version's passphrase is sealed to every device's public key
  (passphrase_shares), so a passphrase set, changed or restored on one device reaches all the others by itself.
- Changing re-encrypts every key from the current passphrase's key to the new one, in one transaction; restoring an
  earlier version does the same towards it. Any device can do either, since every device receives every version.
- Each device keeps the passphrase and its key (HOME/secret.key, protected), so Settings can show it masked, and
  notes which version it has: a device that isn't on the current version neither sends nor reads keys until it has
  taken the current one, so it can't overwrite keys with an old passphrase.
- Fingerprints (key_fingerprint, password_fingerprint, a version's fingerprint) are one-way: they show whether two
  devices hold the same thing without revealing it."""
import base64
import hashlib
import hmac
import json
import os

import facedb
import paths

KEY_FILE = paths.HOME / "secret.key"
DEVICE_KEY = paths.HOME / "device-key.json"
CHECK = b"face-recognizer passphrase check"
SCRYPT = {"n": 2 ** 15, "r": 8, "p": 1}
MIN_PASSPHRASE = 10


class WrongPassphrase(ValueError):
    pass


# --- primitives --------------------------------------------------------------------------------------------------

def _derive(passphrase, salt, n, r, p):
    return hashlib.scrypt(passphrase.encode(), salt=bytes(salt), n=n, r=r, p=p, maxmem=2 ** 27, dklen=32)


def encrypt(key, name, plain):
    """(nonce, ciphertext) of plain (str or bytes); name is bound to it, so a ciphertext can't be moved to another
    name."""
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    nonce = os.urandom(12)
    data = plain.encode() if isinstance(plain, str) else bytes(plain)
    return nonce, AESGCM(key).encrypt(nonce, data, name.encode())


def decrypt(key, name, nonce, ciphertext):
    from cryptography.exceptions import InvalidTag
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    try:
        return AESGCM(key).decrypt(bytes(nonce), bytes(ciphertext), name.encode())
    except InvalidTag:
        raise WrongPassphrase("This key was encrypted with another passphrase.") from None


def fingerprint(key):
    """A short one-way fingerprint of a passphrase key: XXXX-XXXX-XXXX."""
    h = hmac.new(key, b"face-recognizer key fingerprint", hashlib.sha256).hexdigest()[:12].upper()
    return f"{h[:4]}-{h[4:8]}-{h[8:]}"


def password_fingerprint(password, instance):
    """The same for the database password, keyed with the database's own id (so it means nothing elsewhere)."""
    h = hmac.new(str(instance).encode(), (password or "").encode(), hashlib.sha256).hexdigest()[:12].upper()
    return f"{h[:4]}-{h[4:8]}-{h[8:]}"


def digest(key, name, plain):
    """A keyed digest of a secret (for the sync's 'did it change' check), never the secret itself."""
    data = plain.encode() if isinstance(plain, str) else bytes(plain)
    return hmac.new(key, name.encode() + b"\0" + data, hashlib.sha256).hexdigest()


# --- this device's key pair, for passphrases sealed to it --------------------------------------------------------

def _device_private():
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    try:
        d = json.loads(DEVICE_KEY.read_text("utf-8"))
        return X25519PrivateKey.from_private_bytes(base64.b64decode(facedb.unprotect(d["private"])))
    except (OSError, ValueError, KeyError, TypeError):
        pass
    k = X25519PrivateKey.generate()
    from cryptography.hazmat.primitives import serialization
    raw = k.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
    facedb.write_private(DEVICE_KEY, {"private": facedb.protect(base64.b64encode(raw).decode())})
    return k


def device_public():
    from cryptography.hazmat.primitives import serialization
    return _device_private().public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def _box_key(shared, eph_pub, recipient_pub):
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    return HKDF(algorithm=hashes.SHA256(), length=32, salt=None,
                info=b"face-recognizer passphrase share" + eph_pub + recipient_pub).derive(shared)


def seal(recipient_pub, plain):
    """plain sealed to a device's X25519 public key: ephemeral public key + nonce + ciphertext."""
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    eph = X25519PrivateKey.generate()
    eph_pub = eph.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    key = _box_key(eph.exchange(X25519PublicKey.from_public_bytes(bytes(recipient_pub))), eph_pub, bytes(recipient_pub))
    nonce = os.urandom(12)
    return eph_pub + nonce + AESGCM(key).encrypt(nonce, plain.encode(), None)


def unseal(sealed):
    from cryptography.exceptions import InvalidTag
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PublicKey
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    sealed = bytes(sealed)
    eph_pub, nonce, ct = sealed[:32], sealed[32:44], sealed[44:]
    key = _box_key(_device_private().exchange(X25519PublicKey.from_public_bytes(eph_pub)), eph_pub, device_public())
    try:
        return AESGCM(key).decrypt(nonce, ct, None).decode()
    except InvalidTag:
        raise WrongPassphrase("This share was sealed to another device.") from None


# --- this device's copy of the passphrase --------------------------------------------------------------------------

def _local():
    try:
        return json.loads(KEY_FILE.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def local_key():
    """This device's passphrase key, or None when it has none."""
    try:
        return base64.b64decode(facedb.unprotect(_local()["key"]))
    except (ValueError, KeyError, TypeError):
        return None


def local_version():
    v = _local().get("version")
    return v if isinstance(v, int) else None


def saved_passphrase():
    """The passphrase this device has ("" if none)."""
    d = _local()
    try:
        return facedb.unprotect(d["passphrase"]) if d.get("passphrase") else ""
    except (ValueError, TypeError):
        return ""


def _keep(passphrase, key, version):
    facedb.write_private(KEY_FILE, {"key": facedb.protect(base64.b64encode(key).decode()),
                                    "passphrase": facedb.protect(passphrase), "version": version})


def forget_local():
    KEY_FILE.unlink(missing_ok=True)


# --- the database's versions ---------------------------------------------------------------------------------------

def current(conn):
    """The current version as a dict (id, salt, n, r, p, nonce, ciphertext, fingerprint), or None."""
    row = conn.execute("""SELECT id, salt, n, r, p, nonce, ciphertext, fingerprint FROM passphrase_versions
                          WHERE current""").fetchone()
    return None if row is None else dict(zip(("id", "salt", "n", "r", "p", "nonce", "ciphertext", "fingerprint"), row))


def _key_for(version, passphrase):
    """The key passphrase makes under version's settings, or WrongPassphrase."""
    key = _derive(passphrase, version["salt"], version["n"], version["r"], version["p"])
    if decrypt(key, "check", version["nonce"], version["ciphertext"]) != CHECK:
        raise WrongPassphrase()
    return key


def _version_passphrase(conn, vid):
    """This device's share of version vid, opened; None when it has none."""
    row = conn.execute("SELECT sealed FROM passphrase_shares WHERE version = %s AND device = %s",
                       (vid, facedb.device()["id"])).fetchone()
    return unseal(row[0]) if row else None


def _new_version(conn, passphrase, note):
    salt = os.urandom(16)
    key = _derive(passphrase, salt, **SCRYPT)
    nonce, ct = encrypt(key, "check", CHECK)
    vid = conn.execute("""INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext, fingerprint,
                                                           note)
                          VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s) RETURNING id""",
                       (facedb.device()["id"], salt, SCRYPT["n"], SCRYPT["r"], SCRYPT["p"], nonce, ct,
                        fingerprint(key), note)).fetchone()[0]
    return vid, key


def _make_current(conn, vid, new_key, old_key):
    """Re-encrypt every key from old_key to new_key and make version vid current (inside the caller's transaction)."""
    if old_key is not None:
        for name, nonce, ct in conn.execute("SELECT name, nonce, ciphertext FROM secrets").fetchall():
            plain = decrypt(old_key, name, nonce, ct)
            n2, c2 = encrypt(new_key, name, plain)
            conn.execute("UPDATE secrets SET nonce = %s, ciphertext = %s WHERE name = %s", (n2, c2, name))
    conn.execute("UPDATE passphrase_versions SET current = false WHERE current AND id <> %s", (vid,))
    conn.execute("UPDATE passphrase_versions SET current = true WHERE id = %s", (vid,))


def share_all(conn, vid, passphrase):
    """Seal version vid's passphrase to every device with a public key that hasn't got it yet."""
    rows = conn.execute("""SELECT d.id::text, d.public_key FROM devices d WHERE d.public_key IS NOT NULL AND NOT EXISTS
                           (SELECT 1 FROM passphrase_shares s WHERE s.version = %s AND s.device = d.id)""",
                        (vid,)).fetchall()
    for dev, pub in rows:
        conn.execute("INSERT INTO passphrase_shares (version, device, sealed) VALUES (%s, %s, %s) "
                     "ON CONFLICT DO NOTHING", (vid, dev, seal(pub, passphrase)))
    return len(rows)


def set_passphrase(conn, passphrase, again=None):
    """Set, enter or change the passphrase from this device.
    - None set yet: it becomes version 1, sealed to every device.
    - The current one: this device takes it (as if it had arrived by itself).
    - Another one: a change, allowed only on a device that has the current passphrase; every key is re-encrypted
      and the new passphrase sealed to every device. again must match (typed twice).
    Returns {ok, what: set|entered|changed, version, fingerprint}."""
    passphrase = passphrase or ""
    if again is not None and again != passphrase:
        raise ValueError("The two passphrases aren't the same.")
    register(conn)
    with conn.transaction():
        conn.execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE")
        cur = current(conn)
        if cur is not None:
            try:
                key = _key_for(cur, passphrase)
                _keep(passphrase, key, cur["id"])
                share_all(conn, cur["id"], passphrase)
                what, vid = "entered", cur["id"]
            except WrongPassphrase:
                old = local_key() if local_version() == cur["id"] else None
                if old is None:
                    raise WrongPassphrase("That isn't the current passphrase, and this device doesn't have the "
                                          "current one to change it from. It arrives by itself from a device that "
                                          "has it; or restore an earlier one.") from None
                if len(passphrase) < MIN_PASSPHRASE:
                    raise ValueError(f"Use a passphrase of at least {MIN_PASSPHRASE} characters.") from None
                vid, key = _new_version(conn, passphrase, "changed")
                _make_current(conn, vid, key, old)
                share_all(conn, vid, passphrase)
                _keep(passphrase, key, vid)
                what = "changed"
        else:
            if len(passphrase) < MIN_PASSPHRASE:
                raise ValueError(f"Use a passphrase of at least {MIN_PASSPHRASE} characters.")
            vid, key = _new_version(conn, passphrase, "set")
            _make_current(conn, vid, key, None)
            share_all(conn, vid, passphrase)
            _keep(passphrase, key, vid)
            what = "set"
    register(conn)
    return {"ok": True, "what": what, "version": vid, "fingerprint": fingerprint(key)}


def restore(conn, vid):
    """Make an earlier version current again (undo a wrong change): every key is re-encrypted to it and every device
    takes it. Needs this device's shares of that version and of the current one (every device receives both)."""
    register(conn)
    with conn.transaction():
        conn.execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE")
        cur = current(conn)
        row = conn.execute("""SELECT id, salt, n, r, p, nonce, ciphertext, fingerprint FROM passphrase_versions
                              WHERE id = %s""", (int(vid),)).fetchone()
        if row is None:
            raise ValueError("No such passphrase version.")
        target = dict(zip(("id", "salt", "n", "r", "p", "nonce", "ciphertext", "fingerprint"), row))
        if cur is not None and cur["id"] == target["id"]:
            raise ValueError("That's the current passphrase already.")
        old_pass = _version_passphrase(conn, cur["id"]) if cur else None
        new_pass = _version_passphrase(conn, target["id"])
        if new_pass is None or (cur is not None and old_pass is None):
            raise WrongPassphrase("This device hasn't received that passphrase; restore it from a device that has.")
        old_key = _key_for(cur, old_pass) if cur else None
        new_key = _key_for(target, new_pass)
        # a new version with the earlier passphrase, so the history shows the restore
        vid2 = conn.execute("""INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext,
                                                                fingerprint, note)
                               SELECT %s, salt, n, r, p, nonce, ciphertext, fingerprint, %s FROM passphrase_versions
                               WHERE id = %s RETURNING id""",
                            (facedb.device()["id"], f"restored version {target['id']}", target["id"])).fetchone()[0]
        _make_current(conn, vid2, new_key, old_key)
        share_all(conn, vid2, new_pass)
        _keep(new_pass, new_key, vid2)
    register(conn)
    return {"ok": True, "what": "restored", "version": vid2, "fingerprint": fingerprint(new_key)}


def register(conn, db_password=None, instance=None):
    """This device's public key and fingerprints in the devices table (each sync step; cheap)."""
    me = facedb.device()["id"]
    key, ver = local_key(), local_version()
    pwfp = password_fingerprint(db_password, instance) if db_password is not None and instance else None
    conn.execute("""UPDATE devices SET public_key = %s, key_version = %s, key_fingerprint = %s,
                    password_fingerprint = coalesce(%s, password_fingerprint) WHERE id = %s""",
                 (device_public(), ver, fingerprint(key) if key else None, pwfp, me))


def catch_up(conn):
    """Take the current passphrase if this device doesn't have it yet (its share, sealed by the device that set it),
    and seal the current one to any device that lacks it. Returns (has the current one, just took it)."""
    cur = current(conn)
    if cur is None:
        return False, False
    took = False
    if local_version() != cur["id"]:
        try:
            p = _version_passphrase(conn, cur["id"])
        except WrongPassphrase:
            p = None
        if p is None:
            return False, False
        _keep(p, _key_for(cur, p), cur["id"])
        took = True
    share_all(conn, cur["id"], saved_passphrase())
    return True, took


def overview(conn):
    """For Settings: the versions (newest first) and each device's state against the current one."""
    cur = current(conn)
    versions = [{"id": i, "when": t.isoformat().replace("+00:00", "Z"), "by": by or "", "fingerprint": fp,
                 "current": c, "note": note}
                for i, t, by, fp, c, note in conn.execute(
                    """SELECT v.id, v.created_at, d.name, v.fingerprint, v.current, v.note FROM passphrase_versions v
                       LEFT JOIN devices d ON d.id = v.created_by ORDER BY v.id DESC""")]
    devices = [{"name": n, "fingerprint": fp or "", "current": bool(cur and v == cur["id"] and fp == cur["fingerprint"]),
                "password": pfp or "", "me": i == facedb.device()["id"]}
               for i, n, v, fp, pfp in conn.execute(
                   "SELECT id::text, name, key_version, key_fingerprint, password_fingerprint FROM devices ORDER BY name")]
    return {"current": cur and {"id": cur["id"], "fingerprint": cur["fingerprint"]}, "versions": versions,
            "devices": devices}


def status(conn=None):
    """{here: this device has a passphrase, set: the database has one} (set is None without a connection)."""
    out = {"here": local_key() is not None, "set": None}
    if conn is not None:
        out["set"] = current(conn) is not None
    return out
