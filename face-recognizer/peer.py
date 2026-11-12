"""Devices show each other their original photos, over the local network (with a shared database, facedb.py).

The database knows which device has which photo (photo_paths, by content hash). A device that needs an original it
doesn't have asks one of those devices for it; that device reads its own file, checks it's still the same photo,
and answers with the photo view's JPEG. The asking device only shows it (in memory, never on disk). Nothing depends
on how each device sees a shared drive: every device answers for its own files.

The peer service (port: the app's tab page port + 3, 8767 by default; FACE_PEER_PORT) is the one part of the app
that listens beyond this computer, so it answers one thing only, a photo by its hash (and video frame), and only:
- over TLS with this device's own certificate, whose SHA-256 is in the database: the asking device checks it, so no
  other machine can pose as this one;
- to requests signed with the cluster key (HMAC-SHA256 of method, path, time and the asking device), a key every
  device derives from the shared database's password and the database's own id: only your devices can sign, and a
  signature older than CLOCK_SKEW seconds is refused (no replay);
- for photos this device's own records have (photos.json), never by path.
Each device records its addresses, port and certificate fingerprint in the devices table (every sync step).
Off with FACE_PEER=off, or the Settings switch "Share my photos with my other devices"."""
import datetime
import hashlib
import hmac
import http.server
import json
import os
import re
import socket
import ssl
import threading
import time
from pathlib import Path

import facedb
import paths

CERT = paths.HOME / "peer-cert.pem"
KEY = paths.HOME / "peer-key.pem"
CONF = paths.HOME / "peer.json"
CLOCK_SKEW = 120                 # seconds a signed request stays valid
TIMEOUT = 8                      # seconds to wait for another device
MAX_PHOTO = 60_000_000           # bytes accepted from another device
PHOTO_RE = re.compile(r"^/photo/([0-9a-f]{40}|path-[0-9a-f]{24}|nofile-[0-9a-f]{24})(?:\?frame=(\d{1,7}))?$")
_key_cache = {}
CURRENT_KEY = None               # the cluster key as of the last sync step (dbsync_home.HomePart.every_step)


# --- settings ------------------------------------------------------------------------------------------------------

def sharing():
    """Whether this device shows its photos to the others (default on, with a shared database)."""
    if os.environ.get("FACE_PEER", "").lower() == "off":
        return False
    try:
        return json.loads(CONF.read_text("utf-8")).get("share", True) is not False
    except (OSError, ValueError):
        return True


def set_sharing(on):
    paths.atomic_write(CONF, json.dumps({"share": bool(on)}).encode())


def port():
    raw = os.environ.get("FACE_PEER_PORT") or ""
    if raw.isdigit():
        return int(raw)
    base = os.environ.get("FACE_PORTS") or ""
    return (int(base) if base.isdigit() else 8764) + 3


# --- this device's certificate -------------------------------------------------------------------------------------

def certificate():
    """(certificate path, key path, SHA-256 of the certificate's DER), made once."""
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import ec
    from cryptography.x509.oid import NameOID
    if not (CERT.exists() and KEY.exists()):
        key = ec.generate_private_key(ec.SECP256R1())
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f"Face Recognizer {facedb.device()['name']}")])
        now = datetime.datetime.now(datetime.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key())
                .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(days=1))
                .not_valid_after(now + datetime.timedelta(days=3650)).sign(key, hashes.SHA256()))
        paths.atomic_write(KEY, key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                                  serialization.NoEncryption()))
        if os.name != "nt":
            os.chmod(KEY, 0o600)
        paths.atomic_write(CERT, cert.public_bytes(serialization.Encoding.PEM))
    der = ssl.PEM_cert_to_DER_cert(CERT.read_text("ascii"))
    return CERT, KEY, hashlib.sha256(der).hexdigest()


# --- the cluster key -----------------------------------------------------------------------------------------------

def cluster_key(conn):
    """HMAC key every device with the shared database's password derives the same: HKDF(password, database id)."""
    login = facedb.saved_login() or {}
    pw = login.get("password") or ""
    if not pw and os.environ.get("FACE_DB"):
        m = re.search(r"password=(\S+)", os.environ["FACE_DB"])
        pw = m.group(1) if m else ""
    inst = str(conn.execute("SELECT id FROM instance").fetchone()[0])
    k = (pw, inst)
    if k not in _key_cache:
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
        _key_cache.clear()
        _key_cache[k] = HKDF(algorithm=hashes.SHA256(), length=32, salt=inst.encode(),
                             info=b"face-recognizer peer requests").derive(pw.encode())
    return _key_cache[k]


def _context(protocol, verify=None):
    """Python's own SSLContext (TLS 1.2 or newer by default). paths.py makes HTTPS check certificates the operating
    system's way: truststore replaces ssl.SSLContext and verifies every handshake against the system's roots. Right
    for websites, wrong here: the peer service wants no client certificate, and the asking device checks the other
    device's own certificate by its SHA-256 from the database. With truststore in place, the original class's
    property setters reach the replaced class and loop, so verify_mode is set through the C base class, as
    truststore itself does."""
    try:
        import truststore._api as t
        ctx, base = t._original_SSLContext(protocol), t._original_super_SSLContext
    except (ImportError, AttributeError):
        ctx, base = ssl.SSLContext(protocol), None
    if verify is not None:
        ctx.check_hostname = False
        if base is not None:
            base.verify_mode.__set__(ctx, verify)
        else:
            ctx.verify_mode = verify
    return ctx


def sign(key, method, path, when, device):
    return hmac.new(key, f"{method}\n{path}\n{when}\n{device}".encode(), hashlib.sha256).hexdigest()


# --- in the devices table ------------------------------------------------------------------------------------------

def register(conn, running):
    """Where this device can be reached (or that it doesn't share)."""
    me = facedb.device()["id"]
    if not running:
        conn.execute("UPDATE devices SET peer_share = false WHERE id = %s", (me,))
        return
    _c, _k, fp = certificate()
    conn.execute("""UPDATE devices SET peer_addresses = %s, peer_port = %s, peer_cert_sha256 = %s, peer_share = true
                    WHERE id = %s""", (addresses(), port(), fp, me))


_addr_cache = {"at": 0.0, "list": []}


def addresses():
    """This computer's LAN addresses (hardware.own_addresses), looked up at most once a minute."""
    if time.time() - _addr_cache["at"] > 60:
        import hardware
        _addr_cache["list"], _addr_cache["at"] = hardware.own_addresses(), time.time()
    return _addr_cache["list"]


# --- the service ---------------------------------------------------------------------------------------------------

class _Handler(http.server.BaseHTTPRequestHandler):
    """One request to the peer service; `service` (a Service) is set on the subclass Service.start makes."""
    server_version = "FaceRecognizerPeer"
    sys_version = ""
    service = None

    def log_message(self, *a):
        pass

    def _deny(self, code, why):
        body = json.dumps({"error": why}).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        try:
            m = PHOTO_RE.match(self.path)
            if not m:
                return self._deny(404, "not found")
            dev = self.headers.get("X-Peer-Device", "")
            when = self.headers.get("X-Peer-Time", "")
            sig = self.headers.get("X-Peer-Sig", "")
            if not when.isdigit() or abs(time.time() - int(when)) > CLOCK_SKEW:
                return self._deny(403, "stale or missing time")
            key = self.service.key_source()
            if key is None or not hmac.compare_digest(sign(key, "GET", self.path, when, dev), sig):
                return self._deny(403, "not signed by a device of this cluster")
            jpg = self.service.provider(m.group(1), int(m.group(2)) if m.group(2) else None)
            if jpg is None:
                return self._deny(404, "this device doesn't have that photo")
            self.send_response(200)
            self.send_header("Content-Type", "image/jpeg")
            self.send_header("Content-Length", str(len(jpg)))
            self.end_headers()
            self.wfile.write(jpg)
        except Exception as e:                    # never let one request stop the service
            try:
                self._deny(500, type(e).__name__)
            except Exception:
                pass

    def do_POST(self):
        self._deny(405, "only GET")


class _Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = os.name != "nt"         # Windows: a second copy must fail, not share the port


class Service:
    """The peer service: answers signed requests for photos this device has. provider(hash, frame) -> JPEG or None."""

    def __init__(self, provider, key_source, bind="0.0.0.0", listen_port=None):
        self.provider, self.key_source = provider, key_source
        self.srv = None
        self.bind, self.listen_port = bind, listen_port if listen_port is not None else port()

    def start(self):
        cert, key, _fp = certificate()
        handler = type("Handler", (_Handler,), {"service": self})
        self.srv = _Server((self.bind, self.listen_port), handler)
        ctx = _context(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        self.srv.socket = ctx.wrap_socket(self.srv.socket, server_side=True)
        threading.Thread(target=self.srv.serve_forever, daemon=True, name="peer service").start()
        return self

    def stop(self):
        if self.srv:
            self.srv.shutdown()
            self.srv.server_close()
            self.srv = None


# --- asking another device -----------------------------------------------------------------------------------------

def holders(conn, photo):
    """[(device name, addresses, port, cert sha256)] of the other sharing devices that have photo, most recently
    seen first."""
    return conn.execute("""SELECT d.name, d.peer_addresses, d.peer_port, d.peer_cert_sha256
                           FROM photo_paths p JOIN devices d ON d.id = p.device
                           WHERE p.hash = %s AND d.id <> %s AND d.peer_share AND d.peer_port IS NOT NULL
                             AND d.peer_cert_sha256 IS NOT NULL
                           ORDER BY d.last_seen DESC""", (photo, facedb.device()["id"])).fetchall()


def _get(addr, port_, fp, path, key):
    """GET path from one device: TLS, its certificate checked against fp, signed. Bytes, or None."""
    ctx = _context(ssl.PROTOCOL_TLS_CLIENT, ssl.CERT_NONE)   # self-signed: checked by its SHA-256 below instead
    with socket.create_connection((addr, port_), timeout=TIMEOUT) as raw:
        with ctx.wrap_socket(raw, server_hostname=addr) as s:
            der = s.getpeercert(binary_form=True)
            if not der or hashlib.sha256(der).hexdigest() != fp:
                raise ssl.SSLError("that device's certificate isn't the one in the database")
            when, me = str(int(time.time())), facedb.device()["id"]
            req = (f"GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nX-Peer-Device: {me}\r\n"
                   f"X-Peer-Time: {when}\r\nX-Peer-Sig: {sign(key, 'GET', path, when, me)}\r\n\r\n")
            s.sendall(req.encode())
            data = b""
            while len(data) < MAX_PHOTO + 65536:
                chunk = s.recv(65536)
                if not chunk:
                    break
                data += chunk
    head, _, body = data.partition(b"\r\n\r\n")
    status = head.split(b"\r\n", 1)[0].split()
    if len(status) < 2 or status[1] != b"200":
        return None
    m = re.search(rb"\r\ncontent-length:\s*(\d+)", head, re.I)
    if m and len(body) != int(m.group(1)):
        return None
    return body or None


def fetch(photo, frame=None):
    """The photo view's JPEG of photo (a frame of a video) from another device that has it: (jpeg, device name),
    or (None, why)."""
    try:
        conn = facedb.connect()
    except Exception as e:
        return None, f"the shared database can't be reached ({e})"
    try:
        found = holders(conn, photo)
        key = cluster_key(conn)
    finally:
        conn.close()
    if not found:
        return None, "no other device that shares its photos has it"
    path = f"/photo/{photo}" + (f"?frame={int(frame)}" if frame is not None else "")
    tried = []
    for name, addrs, port_, fp in found:
        for a in addrs or []:
            try:
                jpg = _get(a, port_, fp, path, key)
                if jpg:
                    return jpg, name
            except (OSError, ssl.SSLError):
                continue
        tried.append(name)
    return None, "it's on " + ", ".join(tried) + ", which can't be reached now"


def own_photo(store, photo, frame):
    """What this device answers with: the photo view's JPEG, if its records have the photo and the file is still
    the same (size). Same scale as the face boxes (face_id.load_image)."""
    import cv2

    import face_id as F
    rec = store.photos.get(photo) or {}
    p = rec.get("path")
    if not p or not Path(p).exists():
        return None
    if rec.get("size") and Path(p).stat().st_size != rec["size"]:
        return None                                    # changed since it was read: not that photo any more
    img = F.load_image(p) if frame is None or not rec.get("video") else F.video_frame(p, frame)
    if img is None:
        return None
    return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
