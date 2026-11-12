"""Other devices' cameras, kept flowing for this device's Cameras page (tracker.py /api/cameras/frames, /readouts).

One relay per watched device holds one connection to that device's peer service (peer.open_stream: TLS, signed) for
as long as anyone watches, reads its frames and its live readout (a JSON part about once a second, between the
frames) as they come and keeps the newest of each. If the connection drops, it connects again at once (the last
picture stays meanwhile). The page asks for every camera's next frames in one short request, and every readout in
another: a browser allows only six connections at a time to one address, so the page's requests must not grow with
the number of cameras. A relay nobody asked for in IDLE seconds closes its connection. Frames are kept in memory
only."""
import json
import re
import threading
import time

IDLE = 60
READOUT_STALE = 5.0          # s: a readout older than this isn't shown (the device stopped sending it)
_relays = {}
_lock = threading.Lock()
_PART = re.compile(rb"Content-Type:\s*([^\r\n]+)\r\nContent-Length:\s*(\d+)\r\n\r\n")


class Relay:
    def __init__(self, device):
        self.device = device
        self.cv = threading.Condition()
        self.seq, self.jpeg, self.at = 0, None, 0.0
        self.readout, self.readout_at = None, 0.0
        self.used = time.time()
        self.error = ""
        self.sock = None
        self.stopped = False
        threading.Thread(target=self._run, daemon=True, name=f"camera relay {device[:8]}").start()

    def wait_next(self, after, timeout=3.0):
        """(seq, jpeg) of the next frame after `after`, waiting up to timeout; jpeg None when none came."""
        self.used = time.time()
        end = time.time() + timeout
        with self.cv:
            while self.seq <= after or self.jpeg is None:
                left = end - time.time()
                if left <= 0:
                    return after, None
                self.cv.wait(left)
            return self.seq, self.jpeg

    def peek(self, after):
        """(seq, jpeg) when there's a frame newer than `after`, else (after, None); never waits."""
        self.used = time.time()
        with self.cv:
            if self.seq > after and self.jpeg is not None:
                return self.seq, self.jpeg
            return after, None

    def live_readout(self):
        """The device's newest readout ({faces, age, frameAge}), or {faces: [], error} when there's none lately."""
        self.used = time.time()
        with self.cv:
            if self.readout is not None and time.time() - self.readout_at < READOUT_STALE:
                return self.readout
        return {"faces": [], "error": self.error or "no readout from that device yet"}

    def _parts(self, sock, first):
        """Yield (content type, body) of each part of a multipart stream (peer.py writes Content-Type, then
        Content-Length, for each part)."""
        buf = first
        while True:
            m = _PART.search(buf)
            while not m:
                chunk = sock.recv(65536)
                if not chunk:
                    return
                buf += chunk
                if len(buf) > 8_000_000:
                    raise OSError("stream out of step")
                m = _PART.search(buf)
            ctype, n, start = m.group(1).strip().lower(), int(m.group(2)), m.end()
            while len(buf) < start + n:
                chunk = sock.recv(max(65536, start + n - len(buf)))
                if not chunk:
                    return
                buf += chunk
            yield ctype, buf[start:start + n]
            buf = buf[start + n:]

    def _run(self):
        import camstream
        import peer
        while not self.stopped:
            if time.time() - self.used > IDLE:
                break
            try:
                self.sock, _ctype, first = peer.open_stream(self.device)
                self.error = ""
                for ctype, body in self._parts(self.sock, first):
                    if ctype.startswith(b"application/json"):
                        try:
                            ro = json.loads(body)
                        except ValueError:
                            continue
                        if isinstance(ro, dict):
                            with self.cv:
                                self.readout, self.readout_at = ro, time.time()
                        continue
                    with self.cv:
                        gap = time.time() - self.at if self.at else 0
                        if 2.0 < gap < 600:          # nothing came from that device for a while: in the log
                            print(f"Camera relay {self.device[:8]}: no frame for {gap:.1f} s")
                        self.seq += 1
                        self.jpeg, self.at = body, time.time()
                        self.cv.notify_all()
                    camstream.news()                  # the Cameras page's one request for every camera
                    if time.time() - self.used > IDLE or self.stopped:
                        break
            except Exception as e:                    # unreachable, refused, dropped: try again shortly
                if str(e)[:200] != self.error:
                    print(f"Camera relay {self.device[:8]}: {e}")
                self.error = str(e)[:200]
                time.sleep(1.0)
            finally:
                if self.sock is not None:
                    try:
                        self.sock.close()
                    except OSError:
                        pass
                    self.sock = None
        with _lock:
            if _relays.get(self.device) is self:
                del _relays[self.device]


def get(device):
    """The relay for device, started when it's first asked for."""
    with _lock:
        r = _relays.get(device)
        if r is None or r.stopped:
            r = _relays[device] = Relay(device)
        r.used = time.time()
        return r


def stop_all():
    with _lock:
        for r in _relays.values():
            r.stopped = True
        _relays.clear()
