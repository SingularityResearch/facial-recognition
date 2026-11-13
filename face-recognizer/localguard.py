"""
localguard.py - keep other websites out of the local tools' web servers.

label_tool.py and tracker.py serve pages on 127.0.0.1. Without a check, any website open
in your browser could send them requests (for example, a hidden form that overwrites your
labels), or use "DNS rebinding" to read their answers. Every request must therefore:

  * name this computer in its Host header (127.0.0.1 or localhost, with the right port), which
    defeats DNS rebinding, and
  * not come from another website's page (the browser's Sec-Fetch-Site: cross-site), and
  * for anything that changes or computes something (POST): carry the custom header
    X-Face-Tool: 1 and, if the browser sends an Origin, come from this tool's own page.
    Browsers won't let another website add a custom header without the server's permission
    (a CORS preflight these servers never grant), so only the tool's own pages can do it.
"""

import http.server
import json
import sys
import traceback

HEADER = "X-Face-Tool"

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
    handler.send_header("Cross-Origin-Resource-Policy", "same-origin")
    handler.send_header("X-Content-Type-Options", "nosniff")


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


def bind(port):
    """A web server socket on 127.0.0.1:port, not serving yet. OSError when the port is taken."""
    return _Server(("127.0.0.1", port), http.server.BaseHTTPRequestHandler)


class Handler(http.server.BaseHTTPRequestHandler):
    """What the three servers' request handlers share: quiet, every answer with page_headers, the request
    checks (refused), and any error no route caught answered with a JSON 500 (do_GET/do_POST). A server's
    handler subclasses this and defines _get and/or _post, starting each with `if self.refused(...): return`."""
    frame_ancestors = None                      # None: itself and the app's tab page; "'none'": never framed
    policy = None                               # what this server's page needs beyond POLICY: {directive: sources}

    def log_message(self, *args):
        pass

    def send(self, body, ctype="application/json", code=200, cache=False):
        """Answer with bytes (of type ctype) or anything JSON can write. cache: the URL names unchanging content."""
        if not isinstance(body, (bytes, bytearray)):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "max-age=31536000, immutable" if cache else "no-store")
        page_headers(self, self.frame_ancestors, self.policy)
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass                                # the page was closed or reloaded mid-request

    def send_json(self, obj, code=200):
        self.send(obj, code=code)

    def body(self, limit, empty=False):
        """The request body (bytes), at most limit - 1 bytes; see body_size. Raises RequestError (a ValueError)."""
        n = body_size(self.headers, limit, empty)
        return self.rfile.read(n) if n else b""

    def refused(self, write):
        """Refuse (403) and return why, or None: see refuse_reason. write: the request changes or starts something."""
        reason = refuse_reason(self.headers, self.server.server_address[1], write)
        if reason:
            self.send({"error": reason}, code=403)
        return reason

    def do_GET(self):
        answer_errors(self, self._get, self.send_json)

    def do_POST(self):
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


def refuse_reason(headers, port, write):
    """Return why a request must be refused, or None if it may proceed."""
    local = {f"127.0.0.1:{port}", f"localhost:{port}"}
    if headers.get("Host", "") not in local:
        return "This tool only answers requests addressed to 127.0.0.1."
    # Browsers say where a request comes from. Another website's page ("cross-site") gets nothing, not even a
    # picture as an <img> (which would tell it the tool runs, and how many faces it has). The app's tab page
    # framing a program on another port of 127.0.0.1 is "same-site"; typing the address, a bookmark or the .bat
    # opening the page is "none"; programs that aren't browsers don't send it.
    if headers.get("Sec-Fetch-Site") == "cross-site":
        return "Requests from other websites are not allowed."
    if write:
        if headers.get(HEADER) != "1":
            return f"Missing the {HEADER} header; requests must come from the tool's own page."
        origin = headers.get("Origin")
        if origin is not None and origin not in {f"http://{h}" for h in local}:
            return "Requests from other websites are not allowed."
    return None
