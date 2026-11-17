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
It also streams this device's live camera, as its tracker page shows it (camstream.py): GET /stream, the same
TLS and signature, a multipart JPEG stream that lasts until the watching device closes it (camrelay.py).
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
import sys
import threading
import time
from pathlib import Path

import facedb
import paths

CERT = paths.HOME / "peer-cert.pem"
KEY = paths.HOME / "peer-key.pem"
CONF = paths.HOME / "peer.json"
CLOCK_SKEW = 120                 # seconds a signed request stays valid
TIMEOUT = 8                      # seconds to wait for another device's answer
CONNECT = 3                      # seconds to wait for one of its addresses to take the connection: a device may list
                                 # addresses the asking one can't reach (a VPN's, another network's), and each was
                                 # waited for 8 s before the next was tried
MAX_PHOTO = 60_000_000           # bytes accepted from another device
STREAM_FOR = 7 * 24 * 3600       # a /stream request lasts until the watching device closes it
BOUNDARY = "facerecframe"
STREAM_READ = 15                 # s a watching device waits for the next part before it takes the stream as gone
                                 # and connects again (it used to wait a week: a device that vanished froze its tile)
UNREACHED = "can't be reached now"   # in fetch()'s reason when no device that has the photo answered at all
READOUT_EVERY = 1.0             # s between the live readouts sent along a /stream (JSON parts between the frames)
PHOTO_RE = re.compile(r"^/photo/([0-9a-f]{40}|path-[0-9a-f]{24}|nofile-[0-9a-f]{24})(?:\?frame=(\d{1,7}))?$")
_key_cache = {}
CURRENT_KEY = None               # the cluster key as of the last sync step (dbsync_home.HomePart.every_step)
RUNNING = False                  # the peer service is up (frames from the tracker page have somewhere to go)


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
        # a certificate's name holds 64 characters at most: a computer with a long name got no certificate at all,
        # and so no peer service (found on a Mac, 2026-11-14). The name is for people; devices go by the SHA-256
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f"Face Recognizer {facedb.device()['name']}"[:64])])
        now = datetime.datetime.now(datetime.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key())
                .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(days=1))
                .not_valid_after(now + datetime.timedelta(days=3650)).sign(key, hashes.SHA256()))
        paths.write_private(KEY, key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                                   serialization.NoEncryption()))
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
    """The addresses other devices can reach this computer at (hardware.reachable_addresses: its own, without the
    container bridges no other machine can reach), looked up at most once a minute."""
    if time.time() - _addr_cache["at"] > 60:
        import hardware
        _addr_cache["list"], _addr_cache["at"] = hardware.reachable_addresses(), time.time()
    return _addr_cache["list"]


_good = {}                       # a device's certificate fingerprint -> its address that answered last


def _order(fp, addrs):
    """A device's addresses in the order to try them: the one that answered last, then those on a network this
    computer is on too, then the rest (each in the order the device listed them)."""
    mine = {a.rpartition(".")[0] for a in addresses()}
    return sorted(addrs or [], key=lambda a: (a != _good.get(fp), a.rpartition(".")[0] not in mine))


def _answered(fp, addr, ok=True):
    """Remember (or forget) which of a device's addresses works."""
    if ok:
        _good[fp] = addr
    elif _good.get(fp) == addr:
        del _good[fp]


# --- the service ---------------------------------------------------------------------------------------------------

class _Handler(http.server.BaseHTTPRequestHandler):
    """One request to the peer service; `service` (a Service) is set on the subclass Service.start makes."""
    server_version = "FaceRecognizerPeer"
    sys_version = ""
    service = None
    timeout = 30         # seconds without a byte moving, the TLS handshake included: a silent connection ends
    disable_nagle_algorithm = True   # an answer's last piece left only when the asker had acknowledged the rest:
                                     # 6 to 26 ms on every photo, on a network that needs 1 (measured 2026-11-14)

    def setup(self):
        # The TLS handshake happens here, on this request's own thread and under the timeout. It used to happen
        # inside accept() on the one serving thread, where a connection that said nothing stopped the whole service.
        self.request.settimeout(self.timeout)
        self.request.do_handshake()
        super().setup()

    def log_message(self, *a):
        pass

    def _deny(self, code, why, **more):
        body = json.dumps({"error": why, **more}).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        try:
            m = PHOTO_RE.match(self.path)
            if not m and self.path not in ("/stream", "/readout"):
                return self._deny(404, "not found")
            dev = self.headers.get("X-Peer-Device", "")
            when = self.headers.get("X-Peer-Time", "")
            sig = self.headers.get("X-Peer-Sig", "")
            if not when.isdigit() or abs(time.time() - int(when)) > CLOCK_SKEW:
                return self._deny(403, "stale or missing time", now=int(time.time()))   # the asker says how far apart
            key = self.service.key_source()
            if key is None or not hmac.compare_digest(sign(key, "GET", self.path, when, dev), sig):
                return self._deny(403, "not signed by a device of this cluster")
            if self.path == "/stream":
                return self._stream(dev)
            if self.path == "/readout":
                import camstream
                body = json.dumps(camstream.readout()).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
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
                self._deny(500, f"{type(e).__name__}: {str(e)[:200]}")
            except Exception:
                pass

    def _stream(self, dev):
        """This device's camera as its tracker page shows it, frame by frame (multipart JPEG)."""
        import camstream
        seq, jpg = camstream.latest()
        if jpg is None:
            return self._deny(404, "this device's camera isn't streaming")
        self.send_response(200)
        self.send_header("Content-Type", f"multipart/x-mixed-replace; boundary={BOUNDARY}")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        end, watcher, said = time.time() + STREAM_FOR, f"peer:{dev}:{id(self)}", 0.0
        while time.time() < end:
            out = b""
            if jpg is not None:
                out += f"--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {len(jpg)}\r\n\r\n".encode() \
                    + jpg + b"\r\n"
            if time.time() - said >= READOUT_EVERY:  # the readout rides along: no request of its own per camera
                ro = json.dumps(camstream.readout()).encode()
                out += f"--{BOUNDARY}\r\nContent-Type: application/json\r\nContent-Length: {len(ro)}\r\n\r\n" \
                    .encode() + ro + b"\r\n"
                said = time.time()
            if out:
                self.wfile.write(out)
                self.wfile.flush()
            seq, jpg = camstream.wait_next(seq, watcher, timeout=READOUT_EVERY)

    def do_POST(self):
        self._deny(405, "only GET")


class _Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = os.name != "nt"         # Windows: a second copy must fail, not share the port
    request_queue_size = 128                      # Python's 5: on Windows, 16 requests at one moment took a second
                                                  # (those that found the queue full were sent again a second later)

    def handle_error(self, request, client_address):
        if not isinstance(sys.exc_info()[1], OSError):   # a failed or silent handshake, a dropped connection: no news
            super().handle_error(request, client_address)


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
        self.srv.socket = ctx.wrap_socket(self.srv.socket, server_side=True, do_handshake_on_connect=False)
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


def _why(body):
    """What a refusing device said; when it was the time of the request, how far apart the two clocks are."""
    text = body.decode("utf-8", "replace")[:300]
    try:
        said = json.loads(text)
        if isinstance(said.get("now"), (int, float)):
            diff = said["now"] - time.time()
            return (f"its clock is {abs(diff):.0f} s {'ahead of' if diff > 0 else 'behind'} this device's (more than "
                    f"{CLOCK_SKEW} s apart is refused): check that both set their clocks automatically")
        return str(said.get("error"))[:100]
    except (ValueError, AttributeError):
        return text[:100]


_clock_said = {}                 # address -> when the log last said that device's clock is far off


def _clock_note(addr, head, now=None):
    """Say in the log, at most once an hour for a device, that its clock and this one's are more than half of
    CLOCK_SKEW apart (from the Date line of its answer, to the second): past CLOCK_SKEW the devices refuse each
    other, and nothing said so beforehand (one device was found 81 s behind, its time service stopped). Returns
    the difference in seconds, or None when the answer has no date."""
    import email.utils
    m = re.search(rb"(?:^|\r\n)date:\s*([^\r\n]+)", head, re.I)
    try:
        theirs = email.utils.parsedate_to_datetime(m.group(1).decode("ascii")).timestamp()
    except (AttributeError, TypeError, ValueError, UnicodeDecodeError):
        return None
    now = time.time() if now is None else now
    diff = theirs - now
    if abs(diff) > CLOCK_SKEW / 2 and now - _clock_said.get(addr, 0) > 3600:
        _clock_said[addr] = now
        print(f"The clock of the device at {addr} is {abs(diff):.0f} s {'ahead of' if diff > 0 else 'behind'} this "
              f"device's. At {CLOCK_SKEW} s apart the devices stop showing each other photos and cameras: check "
              "that both set their clocks automatically.")
    return diff


def _get(addr, port_, fp, path, key):
    """GET path from one device: TLS, its certificate checked against fp, signed. Bytes, or None."""
    ctx = _context(ssl.PROTOCOL_TLS_CLIENT, ssl.CERT_NONE)   # self-signed: checked by its SHA-256 below instead
    with socket.create_connection((addr, port_), timeout=CONNECT) as raw:
        raw.settimeout(TIMEOUT)
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
        if len(status) > 1 and status[1] == b"403":
            raise PermissionError("refused: " + _why(body))
        if len(status) > 1 and status[1] != b"404":
            raise OSError(f"answered {status[1].decode()}: " + body.decode("utf-8", "replace")[:100])
        return None
    _clock_note(addr, head)
    m = re.search(rb"\r\ncontent-length:\s*(\d+)", head, re.I)
    if m and len(body) != int(m.group(1)):
        return None
    return body or None


def device_peer(conn, device_id):
    """(name, addresses, port, cert sha256) of a sharing device, or None."""
    return conn.execute("""SELECT name, peer_addresses, peer_port, peer_cert_sha256 FROM devices
                           WHERE id = %s AND peer_share AND peer_port IS NOT NULL AND peer_cert_sha256 IS NOT NULL""",
                        (device_id,)).fetchone()


def fetch_readout(device_id):
    """Another device's live readout ({faces, age}) through the peer service."""
    found, key = facedb.ask(lambda conn: (device_peer(conn, device_id), cluster_key(conn)))
    if not found:
        raise OSError("that device doesn't share its camera")
    name, addrs, port_, fp = found
    last = "no address"
    for a in _order(fp, addrs):
        try:
            body = _get(a, port_, fp, "/readout", key)
            if body:
                _answered(fp, a)
                return json.loads(body)
        except (OSError, ssl.SSLError, ValueError) as e:
            _answered(fp, a, False)
            last = f"{a}:{port_}: {e}"
    raise OSError(f"{name} can't be reached ({last})")


def open_stream(device_id):
    """Start watching another device's camera: (TLS socket, content type, first bytes of the body) for the caller
    to pass on, or raise OSError with why."""
    found, key = facedb.ask(lambda conn: (device_peer(conn, device_id), cluster_key(conn)))
    if not found:
        raise OSError("that device doesn't share its camera")
    name, addrs, port_, fp = found
    last = "no address"
    for a in _order(fp, addrs):
        try:
            ctx = _context(ssl.PROTOCOL_TLS_CLIENT, ssl.CERT_NONE)
            raw = socket.create_connection((a, port_), timeout=CONNECT)
            raw.settimeout(TIMEOUT)
            s = ctx.wrap_socket(raw, server_hostname=a)
            der = s.getpeercert(binary_form=True)
            if not der or hashlib.sha256(der).hexdigest() != fp:
                s.close()
                raise ssl.SSLError("that device's certificate isn't the one in the database")
            when, me = str(int(time.time())), facedb.device()["id"]
            s.sendall((f"GET /stream HTTP/1.1\r\nHost: {a}\r\nConnection: close\r\nX-Peer-Device: {me}\r\n"
                       f"X-Peer-Time: {when}\r\nX-Peer-Sig: {sign(key, 'GET', '/stream', when, me)}\r\n\r\n").encode())
            head = b""
            while b"\r\n\r\n" not in head and len(head) < 65536:
                chunk = s.recv(4096)
                if not chunk:
                    break
                head += chunk
            head, _, rest = head.partition(b"\r\n\r\n")
            status = head.split(b"\r\n", 1)[0].split()
            if len(status) < 2 or status[1] != b"200":
                s.close()
                if len(status) > 1 and status[1] == b"403":
                    raise PermissionError(f"{name} refused: " + _why(rest))
                raise OSError(f"{name} answered {status[1].decode() if len(status) > 1 else '?'}")
            _clock_note(a, head)
            m = re.search(rb"\r\ncontent-type:\s*([^\r\n]+)", head, re.I)
            s.settimeout(STREAM_READ)                  # the sender writes a readout every second, paused or not
            _answered(fp, a)
            return s, (m.group(1).decode() if m else "multipart/x-mixed-replace"), rest
        except (OSError, ssl.SSLError) as e:
            _answered(fp, a, False)
            last = f"{a}:{port_}: {e}"
    raise OSError(f"{name} can't be reached ({last})")


def fetch(photo, frame=None):
    """The photo view's JPEG of photo (a frame of a video) from another device that has it: (jpeg, device name),
    or (None, why)."""
    try:
        found, key = facedb.ask(lambda conn: (holders(conn, photo), cluster_key(conn)))
    except Exception as e:
        return None, f"the shared database can't be reached ({e})"
    if not found:
        return None, "no other device that shares its photos has it"
    path = f"/photo/{photo}" + (f"?frame={int(frame)}" if frame is not None else "")
    tried, last, answered = [], "", False
    for name, addrs, port_, fp in found:
        for a in _order(fp, addrs):
            try:
                jpg = _get(a, port_, fp, path, key)
                _answered(fp, a)                       # it answered, with the photo or without
                if jpg:
                    return jpg, name
                last, answered = f"{a}:{port_} didn't send it", True
                break                                  # the same device has no other answer at its other addresses
            except PermissionError as e:               # reached, and it refused (the key, the clocks)
                last = f"{a}:{port_}: {e}"
                break
            except (OSError, ssl.SSLError) as e:
                _answered(fp, a, False)
                last = f"{a}:{port_}: {e}"
        tried.append(name)
    # a device that answered without the photo (its file moved or changed) was reached: Describe everything again
    # stops asking only after devices that didn't answer at all (UNREACHED)
    return None, "it's on " + ", ".join(tried) + f", which {'did not send it' if answered else UNREACHED} ({last})"


def own_photo(store, photo, frame):
    """What this device answers with: the photo view's JPEG, if its records have the photo and the file is still
    the same (size). Same scale as the face boxes (face_id.load_image)."""
    import cv2

    import face_id as F
    rec = store.photos.get(photo) or {}
    p, size = rec.get("path"), rec.get("size")
    # A photo this device read itself has its size on record. One without is never shown: the size is how the
    # file is known to still be that photo (and a path is this device's own, never taken from the database:
    # dbsync.PHOTO_FIELDS).
    if not p or not isinstance(p, str) or not isinstance(size, int) or size <= 0 or not Path(p).is_file():
        return None
    if Path(p).stat().st_size != size:
        return None                                    # changed since it was read: not that photo any more
    img = F.load_image(p) if frame is None or not rec.get("video") else F.video_frame(p, frame)
    if img is None:
        return None
    return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
