"""Other devices' cameras, kept flowing for this device's Cameras page (tracker.py /api/cameras/<id>/frame).

One relay per watched device holds one connection to that device's peer service (peer.open_stream: TLS, signed) for
as long as anyone watches, reads its frames as they come and keeps the newest. If the connection drops, it connects
again at once (the last picture stays meanwhile). The page asks for frames one at a time ("the next one after N"),
short requests that never hold a browser connection open: a browser allows only six at a time to one address, and
long-lived streams queued behind each other froze the picture while the readout still came through.
A relay nobody asked for in IDLE seconds closes its connection. Frames are kept in memory only."""
import re
import threading
import time

IDLE = 60
_relays = {}
_lock = threading.Lock()


class Relay:
    def __init__(self, device):
        self.device = device
        self.cv = threading.Condition()
        self.seq, self.jpeg, self.at = 0, None, 0.0
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

    def _frames(self, sock, first):
        """Yield each JPEG of a multipart stream (peer.py writes Content-Length for each part)."""
        buf = first
        while True:
            m = re.search(rb"Content-Length:\s*(\d+)\r\n\r\n", buf)
            while not m:
                chunk = sock.recv(65536)
                if not chunk:
                    return
                buf += chunk
                if len(buf) > 8_000_000:
                    raise OSError("stream out of step")
                m = re.search(rb"Content-Length:\s*(\d+)\r\n\r\n", buf)
            n, start = int(m.group(1)), m.end()
            while len(buf) < start + n:
                chunk = sock.recv(max(65536, start + n - len(buf)))
                if not chunk:
                    return
                buf += chunk
            yield buf[start:start + n]
            buf = buf[start + n:]

    def _run(self):
        import peer
        while not self.stopped:
            if time.time() - self.used > IDLE:
                break
            try:
                self.sock, _ctype, first = peer.open_stream(self.device)
                self.error = ""
                for jpg in self._frames(self.sock, first):
                    with self.cv:
                        self.seq += 1
                        self.jpeg, self.at = jpg, time.time()
                        self.cv.notify_all()
                    if time.time() - self.used > IDLE or self.stopped:
                        break
            except Exception as e:                    # unreachable, refused, dropped: try again shortly
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
