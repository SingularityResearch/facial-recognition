"""Signing the macOS builds without a Mac. Apple signs nothing itself: it gives a certificate for a key that stays
with the developer. This makes that key and the request Apple wants, and later turns Apple's answer into the
repository secrets CI signs and notarizes with (packaging/macos/sign_and_notarize.sh; README: "Signing the macOS app").

  1. python tools/apple_signing.py request --email you@example.com
        HOME/apple-signing/developer-id.key   the private key: stays on this computer, kept as passwords are
        HOME/apple-signing/developer-id.csr   the request: upload it at developer.apple.com, Certificates, "+",
                                              "Developer ID Application" (only the Account Holder can)
     Running it again writes the same request for the same key: a key Apple has certified is never replaced.
  2. python tools/apple_signing.py secrets developerID_application.cer --p8 AuthKey_XXXXXXXXXX.p8 --issuer <id>
        checks the certificate is a Developer ID Application one for this key, and sets the five repository
        secrets with gh: MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD, APPLE_API_KEY_ID, APPLE_API_ISSUER_ID,
        APPLE_API_KEY_P8_BASE64. --check stops before setting them.

Nothing here is written into the repository, and no key or password is printed."""
import argparse
import base64
import re
import secrets as random
import subprocess
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import NameOID

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import paths  # noqa: E402

REPO = "SingularityResearch/facial-recognition"
KIND = "Developer ID Application"            # the certificate for apps given out outside the App Store
SECRETS = ("MACOS_CERT_P12_BASE64", "MACOS_CERT_PASSWORD", "APPLE_API_KEY_ID", "APPLE_API_ISSUER_ID",
           "APPLE_API_KEY_P8_BASE64")


def folder():
    return paths.HOME / "apple-signing"


def load_key(where=None):
    """The private key `request` made, or None."""
    f = (where or folder()) / "developer-id.key"
    if not f.exists():
        return None
    return serialization.load_pem_private_key(paths.unprotect(f.read_text("utf-8")).encode(), None)


def request(email=None, name=None, where=None):
    """Make the key (once) and the certificate request for it. Returns the request's path."""
    where = where or folder()
    key = load_key(where)
    if key is None:
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)     # what Apple issues these for
        pem = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                serialization.NoEncryption()).decode()
        paths.write_private(where / "developer-id.key", paths.protect(pem))
    subject = [x509.NameAttribute(NameOID.COMMON_NAME, name or KIND)]
    if email:
        subject.append(x509.NameAttribute(NameOID.EMAIL_ADDRESS, email))
    csr = x509.CertificateSigningRequestBuilder().subject_name(x509.Name(subject)).sign(key, hashes.SHA256())
    out = where / "developer-id.csr"
    out.write_bytes(csr.public_bytes(serialization.Encoding.PEM))
    return out


def read_certificate(path):
    data = Path(path).read_bytes()
    return x509.load_pem_x509_certificate(data) if data.lstrip().startswith(b"-----") else x509.load_der_x509_certificate(data)


def check(cert, key):
    """The certificate's name ("Developer ID Application: Name (TEAMID)") when it is one of those for this key.
    SystemExit, saying what is wrong, when it isn't."""
    names = cert.subject.get_attributes_for_oid(NameOID.COMMON_NAME)
    name = names[0].value if names else ""
    if not name.startswith(KIND + ":"):
        raise SystemExit(f'This is a "{name or "nameless"}" certificate. CI signs with a "{KIND}" one: make that '
                         f"kind at developer.apple.com (Certificates, +).")
    same = cert.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo) == \
        key.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    if not same:
        raise SystemExit("This certificate is for another key than the one on this computer: it was made from "
                         "another request. Upload developer-id.csr from `request` and download that certificate.")
    return name


def bundle(key, cert, password):
    """The key and its certificate as one .p12, as macOS's `security import` reads it. Its newer versions read the
    modern encryption too, the older ones only this one (what cryptography's documentation gives for them)."""
    how = (serialization.PrivateFormat.PKCS12.encryption_builder().kdf_rounds(50000)
           .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC).hmac_hash(hashes.SHA1())
           .build(password.encode()))
    return pkcs12.serialize_key_and_certificates(b"Developer ID Application", key, cert, None, how)


def api_key(p8, key_id=None, issuer=None):
    """(key id, issuer id, the .p8's text) of an App Store Connect API key, checked for shape."""
    p8 = Path(p8)
    try:
        text = p8.read_bytes().decode("ascii")
        serialization.load_pem_private_key(text.encode(), None)
    except (ValueError, TypeError):
        raise SystemExit(f"{p8.name} isn't an API key file (AuthKey_XXXXXXXXXX.p8 from App Store Connect).") from None
    key_id = key_id or (re.fullmatch(r"AuthKey_([A-Z0-9]+)\.p8", p8.name) or [None, None])[1]
    if not key_id or not re.fullmatch(r"[A-Z0-9]{8,12}", key_id):
        raise SystemExit("Give the key's ID with --key-id (it is in the file's name as Apple names it: AuthKey_<ID>.p8).")
    if not issuer or not re.fullmatch(r"[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}", issuer):
        raise SystemExit("Give the Issuer ID with --issuer: the long id above the list of keys in App Store Connect "
                         "(Users and Access, Integrations, App Store Connect API).")
    return key_id, issuer, text


def values(cer, p8, key_id=None, issuer=None, where=None):
    """{secret name: value} for CI, from Apple's certificate and API key and this computer's private key."""
    key = load_key(where)
    if key is None:
        raise SystemExit("There is no private key on this computer yet: run `request` first, and upload its "
                         "developer-id.csr to get the certificate.")
    cert = read_certificate(cer)
    name = check(cert, key)
    key_id, issuer, text = api_key(p8, key_id, issuer)
    password = random.token_urlsafe(24)
    out = dict(zip(SECRETS, (base64.b64encode(bundle(key, cert, password)).decode(), password, key_id, issuer,
                             base64.b64encode(text.encode()).decode())))
    return name, cert.not_valid_after_utc, out


def set_secrets(vals, repo=REPO, run=subprocess.run):
    for name, value in vals.items():                 # the value on standard input: never on a command line
        run(["gh", "secret", "set", name, "-R", repo], input=value.encode(), check=True)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Signing the macOS builds without a Mac (see the top of this file)")
    sub = ap.add_subparsers(dest="what", required=True)
    r = sub.add_parser("request", help="make the private key and the certificate request to upload to Apple")
    r.add_argument("--email", help="your Apple Account's e-mail address, as Apple's own steps ask for")
    r.add_argument("--name", help="your name or your organization's (Apple puts its own on the certificate)")
    s = sub.add_parser("secrets", help="turn Apple's certificate and API key into the repository's secrets")
    s.add_argument("cer", help="the certificate downloaded from Apple (developerID_application.cer)")
    s.add_argument("--p8", required=True, help="the App Store Connect API key file (AuthKey_XXXXXXXXXX.p8)")
    s.add_argument("--key-id", help="the key's ID, when the file was renamed")
    s.add_argument("--issuer", required=True, help="the Issuer ID shown above the keys in App Store Connect")
    s.add_argument("--repo", default=REPO)
    s.add_argument("--check", action="store_true", help="check everything, set nothing")
    a = ap.parse_args(argv)
    if a.what == "request":
        out = request(a.email, a.name)
        print(f"The certificate request: {out}\n"
              f"The private key stays in {out.parent} (protected; never copy it into the repository).\n"
              f'Upload the request at https://developer.apple.com/account/resources/certificates/add : choose\n'
              f'"{KIND}", then this file. Download the certificate Apple makes and run `secrets` with it.')
        return
    name, until, vals = values(a.cer, a.p8, a.key_id, a.issuer)
    print(f"{name}, valid until {until:%Y-%m-%d}; API key {vals['APPLE_API_KEY_ID']}.")
    if a.check:
        print("All five secrets can be made. Nothing was set (--check).")
        return
    set_secrets(vals, a.repo)
    print(f"Set in {a.repo}: {', '.join(SECRETS)}.\nThe next push builds a signed, notarized macOS app and .dmg.")


if __name__ == "__main__":
    main()
