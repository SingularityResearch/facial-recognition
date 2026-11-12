"""Make TLS certificates for a PostgreSQL server that Face Recognizer devices share (see docs/SETTINGS.md, "Shared
database"): a small certificate authority and a server certificate it signs, for the server's names and addresses.

    python tools/db_certs.py OUTDIR sentient-nas 10.0.0.12 10.0.0.7

writes OUTDIR/ca.crt (give this to every device: it checks the server with it), OUTDIR/ca.key (keep it safe, only to
make a new server certificate later), OUTDIR/server.crt and OUTDIR/server.key (copy both into the server's data
directory, set ssl = on). Nothing here goes into git: OUTDIR must be outside the repository."""
import datetime
import ipaddress
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

YEARS_CA, YEARS_SERVER = 20, 10


def _name(cn):
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn)])


def _pem_key(key):
    return key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                             serialization.NoEncryption())


def make(outdir, names):
    """Write ca.crt/ca.key/server.crt/server.key for `names` (host names and IP addresses) into outdir."""
    out = Path(outdir)
    repo = Path(__file__).resolve().parents[2]
    if out.resolve().is_relative_to(repo):
        raise SystemExit("Put the certificates outside the repository: the keys must never reach git.")
    out.mkdir(parents=True, exist_ok=True)
    now = datetime.datetime.now(datetime.timezone.utc)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca = (x509.CertificateBuilder().subject_name(_name("Face Recognizer database CA")).issuer_name(
        _name("Face Recognizer database CA")).public_key(ca_key.public_key()).serial_number(
        x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=5)).not_valid_after(
        now + datetime.timedelta(days=365 * YEARS_CA)).add_extension(
        x509.BasicConstraints(ca=True, path_length=0), critical=True).add_extension(
        x509.KeyUsage(digital_signature=False, content_commitment=False, key_encipherment=False,
                      data_encipherment=False, key_agreement=False, key_cert_sign=True, crl_sign=True,
                      encipher_only=False, decipher_only=False), critical=True).sign(ca_key, hashes.SHA256()))
    alt = []
    for n in names:
        try:
            alt.append(x509.IPAddress(ipaddress.ip_address(n)))
        except ValueError:
            alt.append(x509.DNSName(n))
    key = ec.generate_private_key(ec.SECP256R1())
    cert = (x509.CertificateBuilder().subject_name(_name(names[0])).issuer_name(ca.subject).public_key(
        key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(
        now - datetime.timedelta(minutes=5)).not_valid_after(now + datetime.timedelta(days=365 * YEARS_SERVER))
        .add_extension(x509.SubjectAlternativeName(alt), critical=False)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .sign(ca_key, hashes.SHA256()))
    (out / "ca.crt").write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    (out / "ca.key").write_bytes(_pem_key(ca_key))
    (out / "server.crt").write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    (out / "server.key").write_bytes(_pem_key(key))
    return out


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    print("Wrote", make(sys.argv[1], sys.argv[2:]))
