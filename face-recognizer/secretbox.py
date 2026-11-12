"""Keys kept in the shared database, encrypted on the device: AES-256-GCM with a key made from the owner's passphrase
(scrypt). The database only ever holds the encrypted keys and, in secret_check, the scrypt settings with a known value
encrypted, to tell a right passphrase from a wrong one. Each device keeps the derived key (never the passphrase) in
HOME/secret.key, protected like the database password (facedb.protect: Windows DPAPI, else a mode-600 file).

Without the passphrase entered on a device, its keys stay in its own files only and nothing about them is sent."""
import base64
import hashlib
import hmac
import json
import os

import facedb
import paths

KEY_FILE = paths.HOME / "secret.key"
CHECK = b"face-recognizer passphrase check"
SCRYPT = {"n": 2 ** 15, "r": 8, "p": 1}
MIN_PASSPHRASE = 10


class WrongPassphrase(ValueError):
    pass


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


def local_key():
    """This device's derived key, or None when the passphrase wasn't entered here."""
    try:
        d = json.loads(KEY_FILE.read_text("utf-8"))
        return base64.b64decode(facedb.unprotect(d["key"]))
    except (OSError, ValueError, KeyError, TypeError):
        return None


def forget_local():
    KEY_FILE.unlink(missing_ok=True)


def digest(key, name, plain):
    """A keyed digest of a secret (for the sync's 'did it change' check), never the secret itself."""
    data = plain.encode() if isinstance(plain, str) else bytes(plain)
    return hmac.new(key, name.encode() + b"\0" + data, hashlib.sha256).hexdigest()


def set_passphrase(conn, passphrase):
    """Enter the owner's passphrase on this device. The first device to do it sets it (secret_check); every other
    device must give the same one (WrongPassphrase otherwise). Keeps the derived key on this device."""
    passphrase = passphrase or ""
    row = conn.execute("SELECT salt, n, r, p, nonce, ciphertext FROM secret_check WHERE id = 1").fetchone()
    if row:
        salt, n, r, p, nonce, ct = row
        key = _derive(passphrase, salt, n, r, p)
        try:
            if decrypt(key, "check", nonce, ct) != CHECK:
                raise WrongPassphrase()
        except WrongPassphrase:
            raise WrongPassphrase("That isn't the passphrase the other devices use.") from None
        first = False
    else:
        if len(passphrase) < MIN_PASSPHRASE:
            raise ValueError(f"Use a passphrase of at least {MIN_PASSPHRASE} characters.")
        salt = os.urandom(16)
        key = _derive(passphrase, salt, **SCRYPT)
        nonce, ct = encrypt(key, "check", CHECK)
        conn.execute("""INSERT INTO secret_check (id, salt, n, r, p, nonce, ciphertext)
                        VALUES (1, %s, %s, %s, %s, %s, %s) ON CONFLICT (id) DO NOTHING""",
                     (salt, SCRYPT["n"], SCRYPT["r"], SCRYPT["p"], nonce, ct))
        first = True
    facedb.write_private(KEY_FILE, {"key": facedb.protect(base64.b64encode(key).decode())})
    return {"ok": True, "first": first}


def status(conn=None):
    """{here: passphrase entered on this device, set: the database has one} (set is None without a connection)."""
    out = {"here": local_key() is not None, "set": None}
    if conn is not None:
        out["set"] = conn.execute("SELECT count(*) FROM secret_check").fetchone()[0] > 0
    return out
