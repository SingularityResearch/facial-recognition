"""HTTPS uses the operating system's certificate checks (paths.py, truststore): with Python's own, a fresh Windows
install couldn't download the face models from GitHub (CERTIFICATE_VERIFY_FAILED, 2026-11-11)."""
import ssl

import paths  # noqa: F401  (importing it switches the checks)


def test_https_uses_the_systems_certificate_checks():
    import truststore
    assert ssl.SSLContext is truststore.SSLContext
    ctx = ssl.create_default_context()
    assert isinstance(ctx, truststore.SSLContext) and ctx.verify_mode == ssl.CERT_REQUIRED
