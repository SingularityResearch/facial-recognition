"""Keys kept in the shared database, encrypted on the device, and the owner's passphrase for them kept in step on
the devices the owner let in (db/006, db/011).

- Keys (Web search, describe) are encrypted with AES-256-GCM under a key made from the passphrase (scrypt); the
  database only ever holds ciphertext.
- Every passphrase ever set is a version (passphrase_versions): its scrypt settings, a known value encrypted with
  its key (to tell a right passphrase from a wrong one), the key's fingerprint, who set it and when. One is current.
- Each device has its own X25519 key pair: the private half in HOME/device-key.json (protected like the database
  password), the public half in devices.public_key.
- The database password alone gets nobody the passphrase (since 1.22.2; before, it was sealed to every row in
  devices). A device gets it in one of two ways, both needing the owner:
  - the owner types it there (set_passphrase: checked against the current version on the device, never sent); or
  - the device asks (ask): it shows a one-time code on its own screen, and the owner types that code on a device
    that has the passphrase (approve). The code proves each side to the other: the asking device's request carries a
    check value made from it over its public key, the answer one over the sealed passphrase. The code never reaches
    the database; a request lasts REQUEST_MINUTES.
- A device that holds the current passphrase shows it (devices.key_proof: a keyed check value over its id and public
  key that only a holder can make). A change or restore is sealed only to devices that show the passphrase it
  replaces, and carries a check value made with that passphrase's key (passphrase_versions.link): a device follows
  a new version only when that fits the passphrase it has. So nobody without the passphrase can hand a device
  another one. A device that missed two changes in a row needs one of the two ways above again.
- Changing re-encrypts every key from the current passphrase's key to the new one, in one transaction; restoring an
  earlier version does the same towards it, on a device that held that version itself (HOME/secret.key "known").
- Each device keeps the passphrase and its key (HOME/secret.key, protected), so Settings can show it masked, and
  notes which version it has: a device that isn't on the current version neither sends nor reads keys until it has
  the current one, so it can't overwrite keys with an old passphrase.
- Fingerprints (key_fingerprint, password_fingerprint, a version's fingerprint) are one-way: they show whether two
  devices hold the same thing without revealing it. They are what a device says of itself; key_proof is the check.

What someone with only the database password can still do: delete or damage the encrypted keys and these tables
(as every other table), which stops keys syncing until the owner sets things right. Not read a key or the passphrase."""
import base64
import hashlib
import hmac
import json
import os
import re
import secrets
import time

import facedb
import paths

KEY_FILE = paths.HOME / "secret.key"
DEVICE_KEY = paths.HOME / "device-key.json"
REQUEST = paths.HOME / "key-request.json"        # this device's open request: its code and the key made from it
CHECK = b"face-recognizer passphrase check"
SCRYPT = {"n": 2 ** 15, "r": 8, "p": 1}
MIN_PASSPHRASE = 10
REQUEST_MINUTES = 15
CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"      # 32 symbols; no I, O, 0 or 1 (read wrongly off a screen)
CODE_LEN = 8                                            # 40 bits, each guess costing CODE_SCRYPT: not in 15 minutes
CODE_SCRYPT = {"n": 2 ** 16, "r": 8, "p": 1}


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


# --- check values only a holder of a key can make -----------------------------------------------------------------

def _mac(key, purpose, *parts):
    """HMAC-SHA256 under a key made from key for this one purpose, over parts (each with its length, so two lists
    of parts never read the same)."""
    sub = hmac.new(key, b"face-recognizer " + purpose, hashlib.sha256).digest()
    h = hmac.new(sub, b"", hashlib.sha256)
    for p in parts:
        b = p if isinstance(p, (bytes, bytearray, memoryview)) else str(p).encode()
        h.update(len(b).to_bytes(4, "big") + bytes(b))
    return h.digest()


def device_proof(key, device, public):
    """What a device holding key writes into devices.key_proof: bound to its id and public key."""
    return _mac(key, b"device proof", device, public)


def _link(prev_key, vid, fp):
    """What the device making version vid writes into its link: made with the key of the version it replaces."""
    return _mac(prev_key, b"version link", vid, fp)


def _same(a, b):
    return a is not None and b is not None and hmac.compare_digest(bytes(a), bytes(b))


# --- this device's copy of the passphrase --------------------------------------------------------------------------

def _local():
    try:
        d = json.loads(KEY_FILE.read_text("utf-8"))
        return d if isinstance(d, dict) else {}
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


def _known():
    """{version id: fingerprint} of every version this device held itself (the ones it may restore)."""
    k = _local().get("known")
    return k if isinstance(k, dict) else {}


def _keep(passphrase, key, version):
    facedb.write_private(KEY_FILE, {"key": facedb.protect(base64.b64encode(key).decode()),
                                    "passphrase": facedb.protect(passphrase), "version": version,
                                    "known": {**_known(), str(version): fingerprint(key)}})


def forget_local():
    KEY_FILE.unlink(missing_ok=True)


def on_current(cur):
    """This device holds the current version's key (cur: current(conn))."""
    key = local_key()
    return bool(cur and key and local_version() == cur["id"]
                and hmac.compare_digest(fingerprint(key), str(cur["fingerprint"])))


# --- the database's versions ---------------------------------------------------------------------------------------

VERSION_COLS = ("id", "salt", "n", "r", "p", "nonce", "ciphertext", "fingerprint", "prev", "link")


def current(conn):
    """The current version as a dict (VERSION_COLS), or None."""
    row = conn.execute(f"SELECT {', '.join(VERSION_COLS)} FROM passphrase_versions WHERE current").fetchone()
    return None if row is None else dict(zip(VERSION_COLS, row))


def _key_for(version, passphrase):
    """The key passphrase makes under version's settings, or WrongPassphrase. The settings come from the database:
    ones no version of this app would have written are refused, not run."""
    n, r, p = version["n"], version["r"], version["p"]
    if not (isinstance(n, int) and 2 ** 14 <= n <= 2 ** 17 and n & (n - 1) == 0 and r == 8 and p == 1):
        raise WrongPassphrase("This passphrase version's settings aren't ones this app makes.")
    key = _derive(passphrase, version["salt"], n, r, p)
    if decrypt(key, "check", version["nonce"], version["ciphertext"]) != CHECK:
        raise WrongPassphrase()
    return key


def _version_passphrase(conn, vid):
    """This device's share of version vid, opened; None when it has none."""
    row = conn.execute("SELECT sealed FROM passphrase_shares WHERE version = %s AND device = %s",
                       (vid, facedb.device()["id"])).fetchone()
    return unseal(row[0]) if row else None


def _new_version(conn, passphrase, note, prev=None, prev_key=None):
    """A new version (not yet current). prev, prev_key: the version it replaces and that one's key, for its link."""
    salt = os.urandom(16)
    key = _derive(passphrase, salt, **SCRYPT)
    nonce, ct = encrypt(key, "check", CHECK)
    vid = conn.execute("""INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext, fingerprint,
                                                           note, prev)
                          VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s) RETURNING id""",
                       (facedb.device()["id"], salt, SCRYPT["n"], SCRYPT["r"], SCRYPT["p"], nonce, ct,
                        fingerprint(key), note, prev)).fetchone()[0]
    if prev_key is not None:
        conn.execute("UPDATE passphrase_versions SET link = %s WHERE id = %s",
                     (_link(prev_key, vid, fingerprint(key)), vid))
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


def holders(conn, key):
    """[(device id, public key)] of the devices that show they hold key (devices.key_proof)."""
    out = []
    for dev, pub, proof in conn.execute("""SELECT id::text, public_key, key_proof FROM devices
                                           WHERE public_key IS NOT NULL AND key_proof IS NOT NULL""").fetchall():
        if _same(proof, device_proof(key, dev, bytes(pub))):
            out.append((dev, bytes(pub)))
    return out


def _put_share(conn, vid, dev, pub, passphrase):
    try:
        sealed = seal(pub, passphrase)
    except ValueError:                               # not a public key
        return False
    conn.execute("""INSERT INTO passphrase_shares (version, device, sealed) VALUES (%s, %s, %s)
                    ON CONFLICT (version, device) DO UPDATE SET sealed = excluded.sealed""", (vid, dev, sealed))
    return True


def share_all(conn, vid, passphrase, held):
    """Seal version vid's passphrase to this device, and to every device that shows it holds the key held (the
    current passphrase's, or the one vid replaces) and hasn't got vid yet. Never to a device that only has a row in
    devices: the database password alone must not be enough to be sent the passphrase. Returns how many others."""
    me = facedb.device()["id"]
    have = {r[0] for r in conn.execute("SELECT device::text FROM passphrase_shares WHERE version = %s", (vid,))}
    if me not in have:
        _put_share(conn, vid, me, device_public(), passphrase)
    n = 0
    for dev, pub in (holders(conn, held) if held is not None else []):
        if dev != me and dev not in have:
            n += _put_share(conn, vid, dev, pub, passphrase)
    return n


def set_passphrase(conn, passphrase, again=None):
    """Set, enter or change the passphrase from this device.
    - None set yet: it becomes version 1, on this device (the others: type it there, or ask and approve).
    - The current one: this device takes it (the way in for a new device: "entered").
    - Another one: a change, allowed only on a device that has the current passphrase; every key is re-encrypted
      and the new passphrase sealed to every device that shows the current one. again must match (typed twice).
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
                share_all(conn, cur["id"], passphrase, None)
                what, vid = "entered", cur["id"]
            except WrongPassphrase:
                old = local_key() if on_current(cur) else None
                if old is None:
                    raise WrongPassphrase("That isn't the current passphrase. Type the one your other devices have, "
                                          "or get it from one of them with a code.") from None
                if len(passphrase) < MIN_PASSPHRASE:
                    raise ValueError(f"Use a passphrase of at least {MIN_PASSPHRASE} characters.") from None
                vid, key = _new_version(conn, passphrase, "changed", cur["id"], old)
                _make_current(conn, vid, key, old)
                share_all(conn, vid, passphrase, old)
                _keep(passphrase, key, vid)
                what = "changed"
        else:
            if len(passphrase) < MIN_PASSPHRASE:
                raise ValueError(f"Use a passphrase of at least {MIN_PASSPHRASE} characters.")
            vid, key = _new_version(conn, passphrase, "set")
            _make_current(conn, vid, key, None)
            share_all(conn, vid, passphrase, None)
            _keep(passphrase, key, vid)
            what = "set"
    register(conn)
    _drop_request(conn)
    return {"ok": True, "what": what, "version": vid, "fingerprint": fingerprint(key)}


def restore(conn, vid):
    """Make an earlier version current again (undo a wrong change): every key is re-encrypted to it and every device
    that shows the current passphrase takes it. Only on a device that has the current passphrase and held that
    earlier one itself: a version this device only finds in the database could be anyone's."""
    register(conn)
    with conn.transaction():
        conn.execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE")
        cur = current(conn)
        row = conn.execute(f"SELECT {', '.join(VERSION_COLS)} FROM passphrase_versions WHERE id = %s",
                           (int(vid),)).fetchone()
        if row is None:
            raise ValueError("No such passphrase version.")
        target = dict(zip(VERSION_COLS, row))
        if cur is not None and cur["id"] == target["id"]:
            raise ValueError("That's the current passphrase already.")
        if cur is not None and not on_current(cur):
            raise WrongPassphrase("This device doesn't have the current passphrase; restore from a device that has.")
        if _known().get(str(target["id"])) != target["fingerprint"]:
            raise WrongPassphrase("This device never had that passphrase; restore it from a device that had.")
        new_pass = _version_passphrase(conn, target["id"])
        if new_pass is None:
            raise WrongPassphrase("This device hasn't received that passphrase; restore it from a device that has.")
        old_key = local_key() if cur else None
        new_key = _key_for(target, new_pass)
        if fingerprint(new_key) != target["fingerprint"]:
            raise WrongPassphrase("That passphrase version was changed in the database.")
        # a new version with the earlier passphrase, so the history shows the restore
        vid2 = conn.execute("""INSERT INTO passphrase_versions (created_by, salt, n, r, p, nonce, ciphertext,
                                                                fingerprint, note, prev)
                               SELECT %s, salt, n, r, p, nonce, ciphertext, fingerprint, %s, %s FROM passphrase_versions
                               WHERE id = %s RETURNING id""",
                            (facedb.device()["id"], f"restored version {target['id']}", cur and cur["id"],
                             target["id"])).fetchone()[0]
        if old_key is not None:
            conn.execute("UPDATE passphrase_versions SET link = %s WHERE id = %s",
                         (_link(old_key, vid2, target["fingerprint"]), vid2))
        _make_current(conn, vid2, new_key, old_key)
        share_all(conn, vid2, new_pass, old_key)
        _keep(new_pass, new_key, vid2)
    register(conn)
    return {"ok": True, "what": "restored", "version": vid2, "fingerprint": fingerprint(new_key)}


def register(conn, db_password=None, instance=None):
    """This device's public key, fingerprints and proof in the devices table (each sync step; cheap)."""
    me = facedb.device()["id"]
    key, ver, pub = local_key(), local_version(), device_public()
    pwfp = password_fingerprint(db_password, instance) if db_password is not None and instance else None
    conn.execute("""UPDATE devices SET public_key = %s, key_version = %s, key_fingerprint = %s, key_proof = %s,
                    password_fingerprint = coalesce(%s, password_fingerprint) WHERE id = %s""",
                 (pub, ver, fingerprint(key) if key else None, device_proof(key, me, pub) if key else None, pwfp, me))


def _follow(conn, cur):
    """Take the current version when it replaced the one this device has and was made by a device holding that one
    (its link fits this device's key). True when taken."""
    key = local_key()
    if key is None or cur.get("prev") != local_version() or not _same(cur.get("link"),
                                                                       _link(key, cur["id"], cur["fingerprint"])):
        return False
    try:
        p = _version_passphrase(conn, cur["id"])
        new = None if p is None else _key_for(cur, p)
    except ValueError:                               # not sealed to this device, not a share at all, a wrong passphrase
        return False
    if new is None or fingerprint(new) != cur["fingerprint"]:
        return False
    _keep(p, new, cur["id"])
    return True


def _learn(conn):
    """Once, on a device coming from a version of the app before 1.22.2: note the versions it holds shares of as
    its own (it received them under the old rules), so it can still restore them."""
    d = _local()
    key = local_key()
    if "known" in d or key is None:
        return
    known = {}
    cols = VERSION_COLS[:8]
    for row in conn.execute(f"""SELECT {', '.join('v.' + c for c in cols)}, s.sealed FROM passphrase_shares s
                                JOIN passphrase_versions v ON v.id = s.version WHERE s.device = %s""",
                            (facedb.device()["id"],)).fetchall():
        ver = dict(zip(cols, row))
        try:
            if fingerprint(_key_for(ver, unseal(row[-1]))) == ver["fingerprint"]:
                known[str(ver["id"])] = ver["fingerprint"]
        except ValueError:
            pass
    if isinstance(d.get("version"), int):
        known[str(d["version"])] = fingerprint(key)
    facedb.write_private(KEY_FILE, {**d, "known": known})


def catch_up(conn):
    """Take the current passphrase if this device doesn't have it yet and may: a change made by a device that held
    the one it has (_follow), or the answer to its own request (_take_answer). Then seal the current one to any
    device that shows it holds it and has no share yet. Returns (has the current one, just took it)."""
    cur = current(conn)
    if cur is None:
        return False, False
    _learn(conn)
    took = False
    if not on_current(cur):
        took = _follow(conn, cur) or _take_answer(conn, cur)
        if not took:
            return False, False
    if REQUEST.exists():
        _drop_request(conn)
    share_all(conn, cur["id"], saved_passphrase(), local_key())
    return True, took


# --- a device without the passphrase asks; the owner approves it on one that has it --------------------------------

def _code_key(code, salt):
    return _derive(code, salt, **CODE_SCRYPT)


def _norm(code):
    return re.sub(r"[^A-Z0-9]", "", str(code or "").upper())


def request():
    """This device's open request: {code: "XXXX-XXXX", until: UTC time} while it lasts, else None."""
    try:
        d = json.loads(REQUEST.read_text("utf-8"))
        if d["until"] > time.time():
            code = facedb.unprotect(d["code"])
            return {"code": f"{code[:4]}-{code[4:]}",
                    "until": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(d["until"]))}
    except (OSError, ValueError, KeyError, TypeError):
        return None
    REQUEST.unlink(missing_ok=True)
    return None


def _request_key():
    try:
        d = json.loads(REQUEST.read_text("utf-8"))
        return base64.b64decode(facedb.unprotect(d["k"])) if d["until"] > time.time() else None
    except (OSError, ValueError, KeyError, TypeError):
        return None


def _drop_request(conn):
    REQUEST.unlink(missing_ok=True)
    conn.execute("DELETE FROM key_requests WHERE device = %s", (facedb.device()["id"],))


def ask(conn):
    """This device asks for the passphrase: a new one-time code, shown only here, and a request in the database with
    a check value made from the code over this device's id and public key. Returns request()."""
    register(conn)
    cur = current(conn)
    if cur is None:
        raise ValueError("No passphrase has been set yet: set one on this device.")
    if on_current(cur):
        raise ValueError("This device has the passphrase already.")
    me, pub = facedb.device()["id"], device_public()
    code = "".join(secrets.choice(CODE_ALPHABET) for _ in range(CODE_LEN))
    salt = os.urandom(16)
    k = _code_key(code, salt)
    conn.execute("""INSERT INTO key_requests (device, public_key, salt, mac) VALUES (%s, %s, %s, %s)
                    ON CONFLICT (device) DO UPDATE SET public_key = excluded.public_key, salt = excluded.salt,
                    mac = excluded.mac, asked_at = now(), version = NULL, sealed = NULL, answer_mac = NULL,
                    answered_by = NULL, answered_at = NULL""", (me, pub, salt, _mac(k, b"key request", me, pub)))
    facedb.write_private(REQUEST, {"code": facedb.protect(code), "k": facedb.protect(base64.b64encode(k).decode()),
                                   "until": time.time() + REQUEST_MINUTES * 60})
    return request()


def waiting(conn):
    """The other devices asking now: [{id, name, asked}]."""
    return [{"id": i, "name": n, "asked": t.isoformat().replace("+00:00", "Z")} for i, n, t in conn.execute(
        """SELECT r.device::text, d.name, r.asked_at FROM key_requests r JOIN devices d ON d.id = r.device
           WHERE r.sealed IS NULL AND r.asked_at > now() - make_interval(mins => %s) AND r.device <> %s
           ORDER BY r.asked_at""", (REQUEST_MINUTES, facedb.device()["id"]))]


def approve(conn, code, device=None):
    """The owner typed, on this device, the code a waiting device shows: check the code against that device's
    request (so the public key in it is the one of the device showing the code), then answer with the passphrase
    sealed to that key and a check value made from the code (so the asking device knows the answer came from where
    its code was typed). Returns {ok, what: "approved", given: its name}."""
    code = _norm(code)
    if len(code) != CODE_LEN:
        raise ValueError(f"The code has {CODE_LEN} letters and digits.")
    with conn.transaction():
        conn.execute("LOCK TABLE passphrase_versions IN EXCLUSIVE MODE")
        cur = current(conn)
        if not on_current(cur):
            raise WrongPassphrase("This device doesn't have the current passphrase itself.")
        rows = conn.execute("""SELECT r.device::text, d.name, r.public_key, r.salt, r.mac FROM key_requests r
                               JOIN devices d ON d.id = r.device
                               WHERE r.sealed IS NULL AND r.asked_at > now() - make_interval(mins => %s)
                               AND r.device <> %s ORDER BY r.asked_at DESC LIMIT 8""",
                            (REQUEST_MINUTES, facedb.device()["id"])).fetchall()
        rows = [r for r in rows if device in (None, "", r[0])]
        if not rows:
            raise ValueError(f"No device is waiting (a code lasts {REQUEST_MINUTES} minutes): ask again on that device.")
        for dev, name, pub, salt, mac in rows:
            k = _code_key(code, bytes(salt))
            if not _same(mac, _mac(k, b"key request", dev, bytes(pub))):
                continue
            passphrase = saved_passphrase()
            try:
                sealed = seal(bytes(pub), passphrase)
            except ValueError:
                continue
            conn.execute("""UPDATE key_requests SET version = %s, sealed = %s, answer_mac = %s, answered_by = %s,
                            answered_at = now() WHERE device = %s""",
                         (cur["id"], sealed, _mac(k, b"key answer", cur["id"], cur["fingerprint"], dev, sealed),
                          facedb.device()["id"], dev))
            conn.execute("""INSERT INTO passphrase_shares (version, device, sealed) VALUES (%s, %s, %s)
                            ON CONFLICT (version, device) DO UPDATE SET sealed = excluded.sealed""",
                         (cur["id"], dev, sealed))
            return {"ok": True, "what": "approved", "given": name}
    raise WrongPassphrase("That isn't the code the waiting device shows.")


def _take_answer(conn, cur):
    """Take the answer to this device's request when it carries the check value only someone who was given this
    device's code can make. True when taken."""
    k = _request_key()
    if k is None:
        return False
    me = facedb.device()["id"]
    row = conn.execute("SELECT version, sealed, answer_mac FROM key_requests WHERE device = %s", (me,)).fetchone()
    if row is None or row[1] is None or row[0] != cur["id"]:
        return False
    sealed = bytes(row[1])
    if not _same(row[2], _mac(k, b"key answer", cur["id"], cur["fingerprint"], me, sealed)):
        return False
    try:
        p = unseal(sealed)
        key = _key_for(cur, p)
    except ValueError:
        return False
    if fingerprint(key) != cur["fingerprint"]:
        return False
    _keep(p, key, cur["id"])
    _drop_request(conn)
    return True


# --- for Settings ---------------------------------------------------------------------------------------------------

def overview(conn):
    """For Settings: the versions (newest first), each device's state against the current one, whether this device
    has it (here), whether it is asking (asking), and who is asking (waiting; only told to a device that can
    approve). A device counts as current when it shows it holds the passphrase (key_proof), which this device can
    check only when it has it too; otherwise by what the device says of itself."""
    cur = current(conn)
    here = on_current(cur)
    key = local_key() if here else None
    known = _known()
    versions = [{"id": i, "when": t.isoformat().replace("+00:00", "Z"), "by": by or "", "fingerprint": fp,
                 "current": c, "note": note, "known": known.get(str(i)) == fp}
                for i, t, by, fp, c, note in conn.execute(
                    """SELECT v.id, v.created_at, d.name, v.fingerprint, v.current, v.note FROM passphrase_versions v
                       LEFT JOIN devices d ON d.id = v.created_by ORDER BY v.id DESC""")]
    devices = []
    for i, n, v, fp, pfp, pub, proof in conn.execute(
            """SELECT id::text, name, key_version, key_fingerprint, password_fingerprint, public_key, key_proof
               FROM devices ORDER BY name""").fetchall():
        says = bool(cur and v == cur["id"] and fp == cur["fingerprint"])
        shown = says and (key is None or (pub is not None and _same(proof, device_proof(key, i, bytes(pub)))))
        devices.append({"name": n, "fingerprint": fp or "", "current": shown, "unproved": says and not shown,
                        "password": pfp or "", "me": i == facedb.device()["id"]})
    return {"current": cur and {"id": cur["id"], "fingerprint": cur["fingerprint"]}, "versions": versions,
            "devices": devices, "here": here, "asking": request() is not None,
            "waiting": waiting(conn) if here else []}


def status(conn=None):
    """{here: this device has a passphrase, set: the database has one} (set is None without a connection)."""
    out = {"here": local_key() is not None, "set": None}
    if conn is not None:
        out["set"] = current(conn) is not None
    return out
