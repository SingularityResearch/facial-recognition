"""
localguard.py - keep other websites out of the local tools' web servers.

label_tool.py and tracker.py serve pages on 127.0.0.1. Without a check, any website open
in your browser could send them requests (for example, a hidden form that overwrites your
labels), or use "DNS rebinding" to read their answers. Every request must therefore:

  * name this computer in its Host header (127.0.0.1 or localhost, with the right port), which
    defeats DNS rebinding, and
  * not come from another website's page (the browser's Sec-Fetch-Site: cross-site), and
  * for data and anything a GET starts (OWN_ONLY: /api/, face pictures, photos): come from the tool's own page
    (the browser says same-origin for those), not from a page of another program on this computer, and
  * for anything that changes or computes something (POST): carry the custom header
    X-Face-Tool: 1 and, if the browser sends an Origin, come from this tool's own page.
    Browsers won't let another website add a custom header without the server's permission
    (a CORS preflight these servers never grant), so only the tool's own pages can do it.
"""

import http.server
import json
import sys
import traceback
from pathlib import Path

HEADER = "X-Face-Tool"
OWN_ONLY = ("/api/", "/thumb/", "/photo/")       # GET paths only the tool's own page may ask for (refuse_reason)
ICON = Path(__file__).resolve().parent / "icons" / "icon.ico"    # the app's icon (tools/make_icon.py)

# Pages may be shown in a frame only by themselves and, when app.py started the program in its window, by the
# window's tab page: app.py adds the tab page's address here (shell_server) before it starts the programs. A
# program started on its own (recognize.bat, label.bat, `python tracker.py`) trusts no other page, so another
# local program on the tab page's usual port (8764) can't frame it. Any other site, or another local program,
# framing a page could trick clicks on it, or pose as the Settings tab (settings_bridge.js).
FRAME_PARENTS = set()

# What a page may load (Content-Security-Policy): only from its own server, with exactly the exceptions the pages
# use. No inline or eval'd script (the page scripts are files and set their handlers in code), so a name or text that
# slipped past esc() still couldn't run. Styles may be inline (<style> blocks and style attributes); pictures may also
# be blob: pictures the page makes itself (the labeling tool's Web search and Copy face). A server adds what only its
# page needs (Handler.policy): the tracker its microphone worklet and Open video (blob:), the app's tab page the two
# programs' frames.
POLICY = {
    "default-src": "'self'",
    "script-src": "'self'",
    "style-src": "'self' 'unsafe-inline'",
    "img-src": "'self' blob:",
    "connect-src": "'self'",
    "object-src": "'none'",
    "base-uri": "'none'",
    "form-action": "'self'",
}


def policy(frame_ancestors=None, extra=None):
    """The Content-Security-Policy value: POLICY with a server's extra directives (which replace the shared ones)
    and frame-ancestors (default: itself and the app's tab page, when it started this program)."""
    rules = {**POLICY, **(extra or {})}
    rules["frame-ancestors"] = frame_ancestors or " ".join(["'self'", *sorted(FRAME_PARENTS)])
    return "; ".join(f"{k} {v}" for k, v in rules.items())


def page_headers(handler, frame_ancestors=None, extra=None):
    """Headers every answer gets: what the page may load and who may frame it (policy), no content sniffing, and
    no reading of the answer by another site's page (Cross-Origin-Resource-Policy: e.g. a face picture as an
    <img> on another website)."""
    handler.send_header("Content-Security-Policy", policy(frame_ancestors, extra))
    if getattr(handler, "permissions", None):       # which pages may use the camera and microphone through this one
        handler.send_header("Permissions-Policy", handler.permissions)
    handler.send_header("Cross-Origin-Resource-Policy", "same-origin")
    handler.send_header("X-Content-Type-Options", "nosniff")
    # Each server's pages in a process of their own (Chromium: WebView2, Qt WebEngine). The three servers are one
    # "site" to a browser (127.0.0.1, whatever the port), so the app window ran its tab page and every tab in one
    # process, on one main thread: a Labeling page kept busy took the tracker from 46 frames read a second to 0.5
    # (a test copy, 2026-11-19; 57 with this header), and a real one with thousands of faces drawn took the camera
    # to 1 frame a second. The tab page checks that each tab's page still runs (app_ui.js silentTabs).
    handler.send_header("Origin-Agent-Cluster", "?1")


class RequestError(ValueError):
    """A request this server can't take, with the HTTP status to answer (400 bad, 413 too large)."""

    def __init__(self, message, code=400):
        super().__init__(message)
        self.code = code


def body_size(headers, limit, empty=False):
    """The request body's length from Content-Length, checked: a number, not negative (which would read until the
    connection closes), below limit (413 otherwise) and, unless empty=True, not 0. Raises RequestError."""
    raw = (headers.get("Content-Length") or "0").strip()
    if not (raw.isascii() and raw.isdigit()):   # also refuses "-1" and "+5"
        raise RequestError("bad Content-Length")
    n = int(raw)
    if n >= limit:
        raise RequestError(f"request too large (limit {limit:,} bytes)", 413)
    if n == 0 and not empty:
        raise RequestError("empty request")
    return n


class _Server(http.server.ThreadingHTTPServer):
    # Windows lets a second program bind a port that's already in use when SO_REUSEADDR is set (Python's HTTP
    # server sets it), and the old program keeps getting the connections. Without it the bind fails as it
    # should, and the caller's "already running" branch is taken.
    allow_reuse_address = sys.platform != "win32"
    request_queue_size = 128                    # Python's 5 is few for a page that asks many things at one moment


def bind(port):
    """A web server socket on 127.0.0.1:port, not serving yet. OSError when the port is taken."""
    return _Server(("127.0.0.1", port), http.server.BaseHTTPRequestHandler)


class Handler(http.server.BaseHTTPRequestHandler):
    """What the three servers' request handlers share: quiet, every answer with page_headers, the request
    checks (refused), and any error no route caught answered with a JSON 500 (do_GET/do_POST). A server's
    handler subclasses this and defines _get and/or _post, starting each with `if self.refused(...): return`."""
    frame_ancestors = None                      # None: itself and the app's tab page; "'none'": never framed
    policy = None                               # what this server's page needs beyond POLICY: {directive: sources}
    # Connections are kept open (HTTP/1.1) and used for one request after another. With Python's default, HTTP/1.0,
    # every request was a new connection: about 90 a second from the tracker page. On Windows about one in a
    # thousand of those got no answer and failed after a second (found 2026-11-13): the browser and the server each
    # close a finished connection at the same moment, Windows keeps the closed pair of ports for a while, and a new
    # connection that is handed the same local port again is not let in. Kept open, a page uses a handful of
    # connections for hours. Every answer must say its length (send does), and a request body that wasn't read
    # must close the connection (send does that too): its bytes would be taken for the next request.
    protocol_version = "HTTP/1.1"
    timeout = 120                               # a connection nothing was asked on for two minutes is closed
    # An answer goes out in one piece and at once. Written as headers and then body on a kept-open connection, the
    # body waited for the page's acknowledgement of the headers, which a page delays about 40 ms: every request
    # took 40 ms longer and the Linux camera stream fell from 20 to 15 frames a second (measured 2026-11-13).
    wbufsize = 64 * 1024                        # headers and body together (flushed after each request)
    disable_nagle_algorithm = True              # and never held back for an acknowledgement

    def log_message(self, *args):
        pass

    def handle(self):
        try:
            super().handle()
        except ConnectionError:                 # the page closed or dropped a kept-open connection: nothing to say
            pass

    def _body_unread(self):
        """This request came with a body that body() didn't take (refused, too large, or a route without one)."""
        if getattr(self, "_body_taken", False):
            return False
        if self.headers.get("Transfer-Encoding"):
            return True
        try:
            return int(self.headers.get("Content-Length") or 0) != 0
        except ValueError:
            return True

    def keep_or_close(self):
        """Before end_headers: close this connection after the answer when the request's body is still unread."""
        if self._body_unread():
            self.close_connection = True
            self.send_header("Connection", "close")

    def send(self, body, ctype="application/json", code=200):
        """Answer with bytes (of type ctype) or anything JSON can write. Never to be kept by the browser
        (no-store): through 1.24.1 face pictures and photos were answered as cacheable for a year, so the
        window's cache on disk kept the picture of a face Ignored or a person forgotten since, and originals
        another device had only shown. A page that wants a picture twice keeps it in memory (label_ui.js)."""
        if not isinstance(body, (bytes, bytearray)):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        page_headers(self, self.frame_ancestors, self.policy)
        self.keep_or_close()
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True        # the page was closed or reloaded mid-request

    def send_json(self, obj, code=200):
        self.send(obj, code=code)

    def favicon(self):
        """/favicon.ico: the app's icon (icons/icon.ico), for a browser's tab when a page is opened there."""
        try:
            self.send(ICON.read_bytes(), "image/x-icon")
        except OSError:
            self.send({"error": "not found"}, code=404)

    def body(self, limit, empty=False):
        """The request body (bytes), at most limit - 1 bytes; see body_size. Raises RequestError (a ValueError)."""
        n = body_size(self.headers, limit, empty)
        data = self.rfile.read(n) if n else b""
        self._body_taken = True
        return data

    def refused(self, write, own=False):
        """Refuse (403) and return why, or None: see refuse_reason. write: the request changes or starts something.
        own: a GET for data or one that starts work, which only the tool's own page may send."""
        reason = refuse_reason(self.headers, self.server.server_address[1], write, own)
        if reason:
            self.send({"error": reason}, code=403)
        return reason

    def do_GET(self):
        self._body_taken = False                # one handler serves a connection's requests one after another
        answer_errors(self, self._get, self.send_json)

    def do_POST(self):
        self._body_taken = False
        answer_errors(self, self._post, self.send_json)

    def _get(self):
        self.send({"error": "not found"}, code=404)

    def _post(self):
        self.send({"error": "not found"}, code=404)


def answer_errors(handler, run, send_error):
    """Run one request; a mistake no route caught still gets an answer (500 with the error), not a dropped
    connection, and is written to the log."""
    try:
        run()
    except ConnectionError:                      # the page went away before the answer was out: nothing to log
        pass
    except Exception as e:
        traceback.print_exc()
        try:
            send_error({"error": f"{type(e).__name__}: {e}"}, 500)
        except OSError:
            pass


def refuse_reason(headers, port, write, own=False):
    """Return why a request must be refused, or None if it may proceed."""
    local = {f"127.0.0.1:{port}", f"localhost:{port}"}
    if headers.get("Host", "") not in local:
        return "This tool only answers requests addressed to 127.0.0.1."
    # Browsers say where a request comes from. Another website's page ("cross-site") gets nothing, not even a
    # picture as an <img> (which would tell it the tool runs, and how many faces it has). The app's tab page
    # framing a program on another port of 127.0.0.1 is "same-site"; typing the address, a bookmark or the .bat
    # opening the page is "none"; programs that aren't browsers don't send it.
    site = headers.get("Sec-Fetch-Site")
    if site == "cross-site":
        return "Requests from other websites are not allowed."
    # A page's requests to its own server are "same-origin". "same-site" is a page on another port of this
    # computer (some other program's page), "none" an address typed or opened by a program: those get the page
    # itself, but no data, and can't start what a GET starts (a search of the network, a camera relay, a look for
    # faces in a photo). Through 1.24.2 they could. A program that isn't a browser sends the header instead.
    if own and site not in (None, "same-origin") and headers.get(HEADER) != "1":
        return "Only this tool's own page may ask for that."
    if write:
        if headers.get(HEADER) != "1":
            return f"Missing the {HEADER} header; requests must come from the tool's own page."
        origin = headers.get("Origin")
        if origin is not None and origin not in {f"http://{h}" for h in local}:
            return "Requests from other websites are not allowed."
    return None
