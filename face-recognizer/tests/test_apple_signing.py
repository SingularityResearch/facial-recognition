"""tools/apple_signing.py: the certificate request for Apple and the repository secrets made from Apple's answer,
with a made-up "Developer ID Application" certificate standing in for Apple's. On macOS (CI) the .p12 is also given
to the system's own `security import`, which is what CI's signing step does with it."""
import base64
import datetime
import subprocess
import sys
from pathlib import Path

import pytest
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import apple_signing as A  # noqa: E402


def certificate(key, name="Developer ID Application: Ann Example (ABCDE12345)"):
    """What Apple would send back for `key`'s request (signed here by the key itself)."""
    who = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    now = datetime.datetime.now(datetime.timezone.utc)
    return (x509.CertificateBuilder().subject_name(who).issuer_name(who).public_key(key.public_key())
            .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=5))
            .not_valid_after(now + datetime.timedelta(days=365))
            .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.CODE_SIGNING]), critical=True)
            .sign(key, hashes.SHA256()))


@pytest.fixture
def signing(tmp_path, monkeypatch):
    """A folder of its own for the key, an API key file, and Apple's made-up certificate for the request."""
    monkeypatch.setattr(A, "folder", lambda: tmp_path / "apple-signing")
    csr = A.request("ann@example.com")
    p8 = tmp_path / "AuthKey_ABC123DEFG.p8"
    from cryptography.hazmat.primitives.asymmetric import ec
    p8.write_bytes(ec.generate_private_key(ec.SECP256R1()).private_bytes(
        serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    cer = tmp_path / "developerID_application.cer"
    cer.write_bytes(certificate(A.load_key()).public_bytes(serialization.Encoding.DER))     # as Apple sends it
    return csr, cer, p8


ISSUER = "69a6de70-03db-47e3-e053-5b8c7c11a4d1"


def test_the_request_is_what_apple_takes_and_the_key_stays_put(signing):
    csr_path, _, _ = signing
    csr = x509.load_pem_x509_csr(csr_path.read_bytes())
    assert csr.is_signature_valid and csr.public_key().key_size == 2048
    assert csr.subject.get_attributes_for_oid(NameOID.EMAIL_ADDRESS)[0].value == "ann@example.com"
    stored = (csr_path.parent / "developer-id.key").read_text()
    assert "PRIVATE KEY" not in stored and stored.split(":")[0] in ("dpapi", "plain")    # kept as passwords are
    before = A.load_key().public_key().public_numbers()
    A.request()                                              # asked again: the same key, never a new one
    assert A.load_key().public_key().public_numbers() == before


def test_apples_certificate_becomes_the_five_secrets(signing):
    _, cer, p8 = signing
    name, until, vals = A.values(cer, p8, issuer=ISSUER)
    assert name == "Developer ID Application: Ann Example (ABCDE12345)" and until.year >= 2026
    assert tuple(vals) == A.SECRETS and vals["APPLE_API_KEY_ID"] == "ABC123DEFG" and vals["APPLE_API_ISSUER_ID"] == ISSUER
    assert base64.b64decode(vals["APPLE_API_KEY_P8_BASE64"]) == p8.read_bytes()
    key, cert, _ = pkcs12.load_key_and_certificates(base64.b64decode(vals["MACOS_CERT_P12_BASE64"]),
                                                    vals["MACOS_CERT_PASSWORD"].encode())
    assert key.public_key().public_numbers() == A.load_key().public_key().public_numbers()
    assert cert.subject == A.read_certificate(cer).subject
    ran = []
    A.set_secrets(vals, "someone/repo", run=lambda cmd, **kw: ran.append((cmd, kw)))
    assert [c[3] for c, _ in ran] == list(A.SECRETS) and all(c[:3] == ["gh", "secret", "set"] for c, _ in ran)
    for (cmd, kw), value in zip(ran, vals.values()):         # the value goes in on standard input, never as an argument
        assert kw["input"] == value.encode() and value not in " ".join(cmd)


def test_the_wrong_things_are_refused_with_what_to_do(signing, tmp_path):
    _, cer, p8 = signing
    other = tmp_path / "other.cer"
    other.write_bytes(certificate(rsa.generate_private_key(public_exponent=65537, key_size=2048)).public_bytes(serialization.Encoding.PEM))
    with pytest.raises(SystemExit, match="another key"):
        A.values(other, p8, issuer=ISSUER)
    wrong_kind = tmp_path / "dev.cer"
    wrong_kind.write_bytes(certificate(A.load_key(), "Apple Development: Ann Example (ABCDE12345)").public_bytes(serialization.Encoding.DER))
    with pytest.raises(SystemExit, match="Developer ID Application"):
        A.values(wrong_kind, p8, issuer=ISSUER)
    with pytest.raises(SystemExit, match="Issuer ID"):
        A.values(cer, p8, issuer="not-an-id")
    renamed = tmp_path / "key.p8"
    renamed.write_bytes(p8.read_bytes())
    with pytest.raises(SystemExit, match="--key-id"):
        A.values(cer, renamed, issuer=ISSUER)
    assert A.values(cer, renamed, key_id="ABC123DEFG", issuer=ISSUER)[2]["APPLE_API_KEY_ID"] == "ABC123DEFG"
    with pytest.raises(SystemExit, match="isn't an API key"):
        A.values(cer, cer, issuer=ISSUER)


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS's own keychain tools")
def test_macos_reads_the_p12_as_the_signing_step_does(signing, tmp_path):
    _, cer, p8 = signing
    _, _, vals = A.values(cer, p8, issuer=ISSUER)
    p12 = tmp_path / "cert.p12"
    p12.write_bytes(base64.b64decode(vals["MACOS_CERT_P12_BASE64"]))
    keychain = tmp_path / "test.keychain-db"

    def security(*args):
        return subprocess.run(["security", *args], capture_output=True, text=True, timeout=60)
    try:
        assert security("create-keychain", "-p", "pw", str(keychain)).returncode == 0
        assert security("unlock-keychain", "-p", "pw", str(keychain)).returncode == 0
        r = security("import", str(p12), "-k", str(keychain), "-P", vals["MACOS_CERT_PASSWORD"], "-T", "/usr/bin/codesign")
        assert r.returncode == 0, r.stderr
        found = security("find-identity", "-p", "codesigning", str(keychain)).stdout       # (not trusted: Apple didn't sign it)
        assert "Developer ID Application: Ann Example (ABCDE12345)" in found
    finally:
        security("delete-keychain", str(keychain))
