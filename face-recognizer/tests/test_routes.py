"""A tripwire on the three servers' routes (the labeling tool, the tracker with its Cameras page, the app's tab
page): every address a page asks for is answered by the server that sent the page, and every route a server
answers has a page that asks for it, or is listed here with who else does.

Why: a route is written in two places that nothing ties together, the page script that asks and the handler that
answers. Moving a handler's routes into other modules (label_tool.make_handler is 700 lines), renaming one, or
taking one out can drop a route without any other test noticing: the page then shows "not found" where a list or
a setting was.

How it reads them:
- The pages: each server is asked for its pages (ENTRY) on a test server, as tests/test_servers.py starts them, and
  for the scripts those pages load. Every quoted text in them that starts with "/" and a letter is an address the
  page asks for (asks_in), whoever calls it and however: so which script belongs to which server comes from what
  the server sends, not from a list here.
- The servers: route strings in the files listed in ROUTE_FILES, where the code compares a path with them, looks one
  up in a table or asks whether a path starts with one (routes_in).
- And for real: every GET a page sends is sent to the test server and must be answered (not "not found", no
  error), except those listed in SKIP with the reason. POSTs are compared by reading only: they change things.

No models, camera, database or network: a connection to anything but the three test servers, or a look-up of a
name, is stopped and fails the test."""
import ast
import errno
import re
import socket
import threading
import webbrowser
from collections import namedtuple
from pathlib import Path

import cv2
import numpy as np
import pytest

import app
import desktop
import face_id as F
import label_tool
import labels as L
import test_servers as S
import tracker
from conftest import make_faces

HERE = Path(__file__).resolve().parents[1]

# ---------------------------------------------------------------------------------------------- what this test is told
# Where each server's routes are written. When a handler hands its routes to another module (the handler calls it),
# add that module's file here: route strings are read from every file listed for the server.
ROUTE_FILES = {"label": ("label_tool.py", "label_settings.py", "localguard.py"),
               "tracker": ("tracker.py", "localguard.py"),
               "shell": ("app.py", "localguard.py")}

# The pages each server sends. Their scripts are found from them. A new page goes here (a test says so).
ENTRY = {"label": ("/",), "tracker": ("/", "/cameras"), "shell": ("/",)}

# How a page script's request goes out, by the function the address is handed to. "fetch": POST when its options
# say so. "body": POST when it is called with a body, GET without (tracker_ui.js alertApi).
SENDS = {"fetch": "fetch", "get": "GET", "got": "GET", "post": "POST", "alertApi": "body",
         "Worker": "GET"}                  # new Worker("/jpeg_worker.js"): the browser loads the worker's script
# Functions that look at an address and send nothing (label_ui.js: is this picture's address a face's?)
LOOKS_ONLY = {"startsWith", "endsWith", "includes", "indexOf"}

# An address built on another server's: new URL("cameras", <the tracker's address>) in app_ui.js, the Cameras tab's
# frame. The text is all the script has, so whose address it is built on is said here.
BUILT_ON = {"cameras": ("tracker", "/cameras")}

# A script two pages share may ask for what only one of them uses: (server, address) -> why that server never
# gets the request.
NOT_FROM_THIS_PAGE = {
    ("label", "/api/signals"): "page_store.js is also the tracker page's: person signals (PageStore.signals) are "
                               "used by tracker_ui.js only, and the labeling tool's settings answer has none to send",
}

# Routes no page asks for, with who does: (server, address) -> who.
OTHER_CALLERS = {
    ("label", "/index.html"): "an address typed or kept as a bookmark: the page under its file-like name",
    ("tracker", "/index.html"): "an address typed or kept as a bookmark: the page under its file-like name",
    ("label", "/favicon.ico"): "the browser asks for a tab's icon by itself",
    ("tracker", "/favicon.ico"): "the browser asks for a tab's icon by itself",
    ("shell", "/favicon.ico"): "the browser asks for a tab's icon by itself",
}
# Entries of OTHER_CALLERS that may already be gone, while a route is being taken out: the test passes before and
# after. Take the entry out of both once it is. (/api/autoname went this way on 2026-11-18.)
BEING_REMOVED = set()

# GETs a page sends that this test does not send, with why (from reading each handler): they would reach the
# network or another program on this computer, or start something. They are compared by reading only.
SKIP = {
    ("label", "/api/hardware/network"): "searches the local network for LM Studio servers (mDNS, every address)",
    ("label", "/api/hardware"): "asks the usual AI ports on this computer and the describe server "
                                "(hardware.ai_apps); tests/test_servers.py asks it once",
    ("label", "/api/lmstudio"): "asks whether LM Studio's server answers on this computer's addresses, and "
                                "searches the network for models it lacks (lmstudio.status, missing_models)",
    ("label", "/api/describe/status"): "asks the describe server (LM Studio on this computer) for its models",
    ("label", "/api/music"): "reads what the Spotify app plays and lists this computer's audio interfaces",
    ("shell", "/update"): "asks GitHub for the latest release",
    ("shell", "/open-release"): "opens the releases page in the web browser",
    ("shell", "/restart-app"): "starts the whole app again",
}
# The same where a library the app needs isn't installed (CI's checks job installs only what the tests need).
NEEDS = {("label", "/api/unknown"): "sklearn"}        # groups look-alike faces (LabelDB._regroup)

# How to ask where the bare address isn't the question: an example for an address with a changing part, what a
# route needs in its query, or a question the route must refuse so that nothing happens (with the answer's
# status). {me} is this device's id.
ASK = {
    ("label", "/thumb/"): "/thumb/0.jpg",
    ("label", "/photo/"): "/photo/0.jpg",
    ("label", "/api/describe/"): "/api/describe/0",
    ("label", "/api/photoinfo/"): "/api/photoinfo/0",
    ("label", "/api/possible/"): ("/api/possible/2", 400),      # face 2's photo isn't on this computer: no models needed
    ("tracker", "/api/cameras/frames"): "/api/cameras/frames?after={me}:0",     # this device's own camera only
    ("tracker", "/api/cameras/readouts"): "/api/cameras/readouts?ids={me}",     # (another's would start a relay to it)
    ("shell", "/restart"): ("/restart?what=nothing", 400),      # no such program: nothing is started
    ("shell", "/open"): "/open?what=nothing",                   # no such folder: none is opened
    ("shell", "/attention"): "/attention?unread=0&flash=0",
}

OWN = {"X-Face-Tool": "1", "Sec-Fetch-Site": "same-origin"}     # what a page's own requests carry (localguard)

# ---------------------------------------------------------------------------------------------- reading the pages
Ask = namedtuple("Ask", "server page line method path dynamic")      # dynamic: path is the fixed start of the address
Route = namedtuple("Route", "method path prefix")                    # method None: the code doesn't say; prefix: startswith

ADDRESS = re.compile(r"""(["'`])(/[A-Za-z0-9_][^"'`\s<>]*)""")       # a quoted text that starts like an address
CALL = re.compile(r"([A-Za-z_$][\w$]*)\s*\($")
XHR = re.compile(r"""\.open\(\s*(["'])(GET|POST)\1\s*,$""")
RELATIVE = re.compile(r"""new URL\(\s*(["'`])([^"'`]*)\1\s*,""")
KEYWORDS = {"if", "while", "for", "return", "switch", "case", "typeof", "await", "void"}


def _string_end(text, i):
    """The index just past the string that opens at text[i] (' " or `; a template's ${...} may hold more)."""
    quote, i = text[i], i + 1
    while i < len(text):
        c = text[i]
        if c == "\\":
            i += 2
            continue
        if c == quote:
            return i + 1
        if quote == "`" and text.startswith("${", i):
            i = _code_end(text, i + 2)[0] + 1
            continue
        if c == "\n" and quote != "`":         # no string: an apostrophe in a comment or a pattern
            return i
        i += 1
    return i


def _code_end(text, i):
    """(the index of the bracket that closes the code starting at text[i], whether a comma came before it at that
    level): strings and inner brackets are stepped over."""
    depth, comma = 0, False
    while i < len(text):
        c = text[i]
        if c in "\"'`":
            i = _string_end(text, i)
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif c == "," and depth == 0:
            comma = True
        i += 1
    return i, comma


def _commented(before):
    """True when `before` (a line up to some point) has opened a // comment: a // with every quote before it closed."""
    k = before.find("//")
    while k >= 0:
        if all(before[:k].count(q) % 2 == 0 for q in "\"'`"):
            return True
        k = before.find("//", k + 2)
    return False


def asks_in(text):
    """What a page file asks its server for: ([(line, method, path, dynamic)], [(line, relative address)],
    [what couldn't be read]). An address is any quoted text that starts with "/" and a letter, reduced to its fixed
    part: up to a "?", or up to the first ${...} (dynamic: the address goes on). How it is sent comes from the
    function it is handed to (SENDS); an address that is handed to nothing (a picture's src, a script tag, a
    function that returns it) is a plain GET."""
    asks, unreadable = [], []
    line_of = lambda i: text.count("\n", 0, i) + 1
    for m in ADDRESS.finditer(text):
        start = m.start()
        if _commented(text[text.rfind("\n", 0, start) + 1:start]):
            continue
        raw = m.group(2)
        cut = min((i for i in (raw.find("?"), raw.find("#"), raw.find("${")) if i >= 0), default=len(raw))
        path = raw[:cut]
        dynamic = raw.startswith("${", cut) or path.endswith("/")
        before = text[:start].rstrip()
        method = "GET"
        call = CALL.search(before[-80:])
        if call and call.group(1) not in KEYWORDS:
            name = call.group(1)
            if name in LOOKS_ONLY:
                continue
            rest, more = _code_end(text, _string_end(text, start))
            rest = text[start:rest]
            how = SENDS.get(name)
            if how is None:
                unreadable.append(f"line {line_of(start)}: {name}({raw!r}...): how does {name}() send it? "
                                  f"Add it to SENDS (or LOOKS_ONLY) in tests/test_routes.py")
                continue
            method = how if how in ("GET", "POST") else \
                "POST" if (how == "body" and more) or (how == "fetch" and re.search(r"""method\s*:\s*["']POST["']""", rest)) \
                else "GET"
        elif XHR.search(before[-40:]):
            method = XHR.search(before[-40:]).group(2)
        asks.append((line_of(start), method, path, dynamic))
    # a request whose address isn't written where this can read it: only a name (a helper's parameter) may stand there
    for m in re.finditer(r"""\bfetch\(\s*|\.open\(\s*["'](?:GET|POST)["']\s*,\s*""", text):
        arg = text[m.end():m.end() + 60]
        if not (re.match(r"""["'`]/[A-Za-z0-9_]""", arg) or re.match(r"[A-Za-z_$][\w$]*\s*[,)]", arg)):
            if not _commented(text[text.rfind("\n", 0, m.start()) + 1:m.start()]):
                unreadable.append(f"line {line_of(m.start())}: a request to {arg.splitlines()[0][:40]!r}: write its "
                                  f"address as one quoted text starting with \"/\"")
    relative = [(line_of(m.start()), m.group(2)) for m in RELATIVE.finditer(text)]
    return asks, relative, unreadable


# ---------------------------------------------------------------------------------------------- reading the servers
METHOD = re.compile(r"(?:^|_)(get|post)(?:_|$)", re.I)       # _get, do_POST, GET_ROUTES: which requests a route is for
LIKE_A_ROUTE = re.compile(r"/(?:[A-Za-z0-9_.~-]+/?)*")
ACCESS_RULES = {"OWN_ONLY"}       # localguard.OWN_ONLY says who may ask for what ("/api/", ...): rules, not routes


def routes_in(source):
    """The routes written in one Python file: {Route}. A route is a text like "/api/x" that the code compares a
    path with (==, in a tuple, a match's case), looks up in a table (a dict's keys, also as ("GET", "/api/x")) or
    tests the start of a path with (startswith: a prefix route). The method is taken from the nearest function or
    table whose name says it (_get, do_POST, POST_ROUTES); a route whose surroundings don't say counts for both.

    This reads the code's text, because a route can't be asked of a handler without running it (a POST changes
    things): so it knows these ways of writing a route and no others. The GETs are also asked for real, below."""
    tree = ast.parse(source)
    given = {}                              # name -> the value it is given (the first time), anywhere in the file
    for node in ast.walk(tree):
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            given.setdefault(node.targets[0].id, node.value)

    def texts(node, seen=()):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return [node.value]
        if isinstance(node, (ast.Tuple, ast.List, ast.Set)):
            return [s for e in node.elts for s in texts(e, seen)]
        if isinstance(node, ast.Dict):
            return [s for k in node.keys if k is not None for s in texts(k, seen)]
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):          # ("/a", "/b") + more
            return texts(node.left, seen) + texts(node.right, seen)
        if isinstance(node, ast.Name) and node.id in given and node.id not in seen and node.id not in ACCESS_RULES:
            return texts(given[node.id], seen + (node.id,))
        return []

    found = set()

    def walk(node, names):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            names = names + (node.name,)
        elif isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name):
            names = names + (node.targets[0].id,)
        method = next((m.group(1).upper() for m in map(METHOD.search, reversed(names)) if m), None)
        said, prefix = [], False
        if isinstance(node, ast.Compare) and len(node.ops) == 1 \
                and isinstance(node.ops[0], (ast.Eq, ast.NotEq, ast.In, ast.NotIn)):
            said = texts(node.left) + texts(node.comparators[0])
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "startswith" \
                and node.args:
            said, prefix = texts(node.args[0]), True
        elif isinstance(node, ast.Dict):
            for key in (k for k in node.keys if k is not None):     # a table: {"/api/x": ...} or {("GET", "/api/x"): ...}
                words = texts(key)
                how = next((w for w in words if w in ("GET", "POST")), method)
                found.update(Route(how, s, False) for s in words if LIKE_A_ROUTE.fullmatch(s))
        elif isinstance(node, ast.MatchValue):
            said = texts(node.value)
        found.update(Route(method, s, prefix) for s in said if LIKE_A_ROUTE.fullmatch(s))
        for child in ast.iter_child_nodes(node):
            walk(child, names)
    walk(tree, ())
    # a table's keys say no method where the table is written; they do where a handler looks a path up in it
    return {r for r in found if r.method or not any(o.method and o[1:] == r[1:] for o in found)}


def server_routes(sources=None):
    """{server: {Route}} from ROUTE_FILES. sources: {file name: text} to read instead of a file (the tests of this
    test hand it a changed copy)."""
    read = lambda f: sources[f] if f in (sources or {}) else (HERE / f).read_text("utf-8")
    return {server: set().union(*(routes_in(read(f)) for f in files)) for server, files in ROUTE_FILES.items()}


def answers(route, ask):
    """True if `route` answers what `ask` asks for."""
    if route.method and route.method != ask.method:
        return False
    if route.prefix:
        return ask.path.startswith(route.path)
    return ask.path == route.path and not ask.dynamic


def unanswered(asks, routes):
    """The asks no route of their server answers (but NOT_FROM_THIS_PAGE)."""
    return [a for a in asks if (a.server, a.path) not in NOT_FROM_THIS_PAGE
            and not any(answers(r, a) for r in routes[a.server])]


def unasked(asks, routes):
    """[(server, route)] no page asks for (but OTHER_CALLERS)."""
    return [(server, r) for server, rs in sorted(routes.items()) for r in sorted(rs, key=lambda r: (r.path, str(r.method)))
            if (server, r.path) not in OTHER_CALLERS and not any(answers(r, a) for a in asks if a.server == server)]


def stale_callers(routes):
    """Entries of OTHER_CALLERS whose route is no longer there (but those being removed)."""
    return sorted(set(OTHER_CALLERS) - {(s, r.path) for s, rs in routes.items() for r in rs} - BEING_REMOVED)


def say(a):
    return f"{a.server}: {a.page} line {a.line} asks {a.method} {a.path}{'...' if a.dynamic else ''}"


# ---------------------------------------------------------------------------------------------- the three test servers
NOW = threading.local()                    # .path: the request a server thread is answering


def watched(handler):
    """handler, noting which GET each of its threads is answering (for `reached` below)."""
    class Watched(handler):
        def do_GET(self):
            NOW.path = self.path
            try:
                super().do_GET()
            finally:
                NOW.path = None
    return Watched


class Servers:
    """The three test servers: .urls {name: address}, and .reached, what answering a request reached for outside
    the test (see the servers fixture)."""

    def __init__(self, urls, reached):
        self.urls, self.reached = urls, reached

    def get(self, server, path):
        """(status, headers, body) of a GET as the server's own page sends it."""
        return S.request(self.urls[server], path, headers=OWN)

    def pages(self):
        """{server: {address: text}}: each server's pages (ENTRY) and the scripts and styles they load from it, as
        the server sends them; also a script that a script loads (a worker: tracker_ui.js and jpeg_worker.js)."""
        out = {}
        for server in self.urls:
            got, todo = {}, list(ENTRY[server])
            while todo:
                path = todo.pop()
                if path in got:
                    continue
                status, headers, body = self.get(server, path)
                assert status == 200, f"{server}: its page file {path} isn't answered ({status})"
                got[path] = body.decode("utf-8")
                if "html" in headers["Content-Type"]:
                    todo += [p for _, m, p, dyn in asks_in(got[path])[0]
                             if m == "GET" and not dyn and p.endswith((".js", ".mjs", ".css", ".html"))]
                elif "javascript" in headers["Content-Type"]:
                    todo += [p for _, m, p, dyn in asks_in(got[path])[0]
                             if m == "GET" and not dyn and p.endswith((".js", ".mjs"))]
            out[server] = got
        return out


def page_asks(pages):
    """([Ask] of every page file of every server, [what couldn't be read]), from what the servers sent (pages)."""
    asks, unreadable = [], []
    for server, files in pages.items():
        asks += [Ask(server, "the address bar or the app's frame", 0, "GET", p, False) for p in ENTRY[server]]
        for page, text in sorted(files.items()):
            found, relative, bad = asks_in(text)
            asks += [Ask(server, page, *a) for a in found]
            unreadable += [f"{server}: {page} {b}" for b in bad]
            for line, rel in relative:
                if rel in BUILT_ON:
                    asks.append(Ask(BUILT_ON[rel][0], f"{server}'s {page}", line, "GET", BUILT_ON[rel][1], False))
                else:
                    unreadable.append(f"{server}: {page} line {line}: new URL({rel!r}, ...): whose address is it "
                                      f"built on? Add it to BUILT_ON in tests/test_routes.py")
    return asks, unreadable


@pytest.fixture
def servers(data, home, tmp_path, monkeypatch, serve):
    """The labeling tool, the tracker and the tab page on test servers, set up as tests/test_servers.py does (a
    made-up library of six faces, no models), plus one real photo so a photo can be asked for. While the test runs,
    a connection to anything but these three servers, a packet to anywhere and a look-up of any name fail and are
    noted in .reached, as is a try to open the browser or a folder or to start anything again: a test never
    reaches the network or another program."""
    photo = tmp_path / "photo0.jpg"
    cv2.imwrite(str(photo), np.full((60, 80, 3), 120, np.uint8))
    face = cv2.imencode(".jpg", np.full((112, 112, 3), 90, np.uint8))[1].tobytes()
    make_faces(data, 6, thumbs=[face] * 6)
    F.save_json("photos.json", {"h0": {"path": str(photo), "name": photo.name, "size": photo.stat().st_size},
                                "h1": {"path": ""}, "h2": {"path": ""}})
    F.Store().save()
    monkeypatch.setattr(F, "load_image", lambda p, data=None: cv2.imread(str(p)))      # (the real one needs Pillow)
    monkeypatch.setattr(F, "WEBCAM", tmp_path / "webcam")                              # as test_servers.track
    monkeypatch.setattr(tracker, "PRESENCE", tmp_path / "presence")
    monkeypatch.setattr(F, "ALERTS", tmp_path / "alerts.json")
    monkeypatch.setattr(app, "WINDOW", None)
    monkeypatch.setitem(app.ALIVE, "at", None)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    urls = {"label": serve(watched(label_tool.make_handler(db, L.Importer(db)))),
            "tracker": serve(watched(tracker.make_handler(S.FakeEngine(tmp_path)))),
            "shell": serve(watched(app.shell_handler()))}

    ports = {int(u.rsplit(":", 1)[1]) for u in urls.values()}
    before = {t.ident for t in threading.enumerate()}      # threads of earlier tests: stopped, but not blamed
    reached = []

    def note(what):
        if threading.get_ident() not in before:
            reached.append(f"{getattr(NOW, 'path', None) or 'a thread started meanwhile'}: {what}")

    def ours(address):
        return isinstance(address, tuple) and address[0] in ("127.0.0.1", "localhost") and address[1] in ports
    connect, connect_ex, sendto = socket.socket.connect, socket.socket.connect_ex, socket.socket.sendto

    def guarded_connect(sock, address, *a):
        if ours(address):
            return connect(sock, address, *a)
        note(f"a connection to {address}")
        raise ConnectionRefusedError(errno.ECONNREFUSED, "tests reach nothing but their own servers")

    def guarded_connect_ex(sock, address, *a):
        if ours(address):
            return connect_ex(sock, address, *a)
        note(f"a connection to {address}")
        return errno.ECONNREFUSED

    def guarded_sendto(sock, payload, *a):
        if ours(a[-1]):
            return sendto(sock, payload, *a)
        note(f"a packet to {a[-1]}")
        raise OSError(errno.ENETUNREACH, "tests reach nothing but their own servers")

    def guarded(lookup):                    # a name looked up is a question to the network already (DNS)
        def look_up(host, *a, **k):
            if host in (None, "", "localhost", "127.0.0.1", "::1"):
                return lookup(host, *a, **k)
            note(f"a look-up of {host}")
            raise socket.gaierror(socket.EAI_NONAME, "tests reach nothing but their own servers")
        return look_up
    monkeypatch.setattr(socket.socket, "connect", guarded_connect)
    monkeypatch.setattr(socket.socket, "connect_ex", guarded_connect_ex)
    monkeypatch.setattr(socket.socket, "sendto", guarded_sendto)
    for name in ("getaddrinfo", "gethostbyname", "gethostbyname_ex", "gethostbyaddr"):
        monkeypatch.setattr(socket, name, guarded(getattr(socket, name)))
    for module, name, what in ((webbrowser, "open", "opening the web browser"),
                               (desktop, "open_folder", "opening a folder"),
                               (app, "restart_soon", "starting the app again"),
                               (app, "run_programs", "starting a program")):
        monkeypatch.setattr(module, name, lambda *a, _what=what, **k: note(_what))
    return Servers(urls, reached)


_SENT = {}                                 # what the servers sent, kept for this file's tests (pages)


@pytest.fixture
def pages(request):
    """{server: {address: text}}: every server's pages and the scripts they load, as the server sends them. Asked of
    the test servers once for the tests in this file (starting three servers takes most of a second, and the page
    files don't change while the tests run)."""
    if not _SENT:
        _SENT.update(request.getfixturevalue("servers").pages())
    return _SENT


# ---------------------------------------------------------------------------------------------- the tripwires
def test_every_address_a_page_asks_for_is_answered_by_the_server_that_sent_the_page(pages):
    """A page script asks for /api/something and its server has no such route (renamed, taken out, or moved to a
    module this test wasn't told about): the page shows an error where a list or a setting was, and nothing else
    notices. The page files are read as the servers send them, the routes from the servers' code."""
    asks, unreadable = page_asks(pages)
    assert not unreadable, "requests this test can't read:\n  " + "\n  ".join(unreadable)
    routes = server_routes()
    missing = unanswered(asks, routes)
    assert not missing, (
        "a page asks for what its server doesn't answer:\n  " + "\n  ".join(sorted({say(a) for a in missing}))
        + "\nAdd the route to the server, or take the request out of the page. If the route is there but written in "
          "another module than " + ", ".join(f"{s}: {' and '.join(f)}" for s, f in ROUTE_FILES.items())
        + ", add that module to ROUTE_FILES in tests/test_routes.py; if it is written in a way routes_in() there "
          "doesn't read, teach it.")
    stale = [f"{s} {p}" for s, p in NOT_FROM_THIS_PAGE
             if not any((a.server, a.path) == (s, p) for a in asks)
             or all(any(answers(r, a) for r in routes[s]) for a in asks if (a.server, a.path) == (s, p))]
    assert not stale, f"NOT_FROM_THIS_PAGE in tests/test_routes.py lists what is answered now, or no longer asked: {stale}"


def test_every_route_a_server_answers_has_a_page_that_asks_for_it_or_a_listed_reason(pages):
    """The other way round: a route nobody calls is code that looks used (it is kept working, reviewed for
    security, moved in a refactor) and isn't. A route for another caller than a page (a browser by itself, the
    app, a tool) is listed in OTHER_CALLERS with who."""
    asks, _ = page_asks(pages)
    routes = server_routes()
    nobody = unasked(asks, routes)
    assert not nobody, (
        "routes no page asks for:\n  "
        + "\n  ".join(f"{s}: {r.method or 'GET/POST'} {r.path}{'...' if r.prefix else ''}" for s, r in nobody)
        + "\nAdd the request to a page, take the route out, or add it to OTHER_CALLERS in tests/test_routes.py with "
          "who asks for it (another program, the app, a tool).")
    gone = stale_callers(routes)
    assert not gone, f"OTHER_CALLERS in tests/test_routes.py lists routes that are no longer there: {gone}"
    assert BEING_REMOVED <= set(OTHER_CALLERS)


def test_every_get_a_page_sends_is_answered_for_real(servers, pages):
    """Reading the code can be fooled (a route written in a way routes_in doesn't know, a module not listed). So
    every GET a page sends is sent to the server as the page sends it and must be answered: 200, not "not found"
    and not an error. Not the ones that would reach the network or another program, or start something (SKIP,
    with why); and where the bare address isn't the question, ASK says how to ask."""
    import camstream
    import facedb
    asks, _ = page_asks(pages)
    me = facedb.device()["id"]
    camstream.reset()
    camstream.put(b"\xff\xd8 a frame")              # this device's camera has a picture: the frames request needn't wait
    wrong, sent = [], set()
    try:
        for server, path, dynamic in sorted({(a.server, a.path, a.dynamic) for a in asks if a.method == "GET"}):
            if (server, path) in SKIP or (server, path) in NOT_FROM_THIS_PAGE:
                continue
            if (server, path) in NEEDS and not _installed(NEEDS[server, path]):
                continue
            how = ASK.get((server, path), None if dynamic else path)
            if how is None:
                wrong.append(f"{server}: GET {path}... has a changing part: give an example in ASK (or a reason in "
                             f"SKIP) in tests/test_routes.py")
                continue
            url, want = how if isinstance(how, tuple) else (how, 200)
            status, _, body = servers.get(server, url.format(me=me))
            sent.add((server, path))
            if status != want:
                wrong.append(f"{server}: GET {url} is answered {status}, not {want}: {body[:120]!r}")
    finally:
        camstream.reset()
    assert not wrong, "a GET a page sends isn't answered:\n  " + "\n  ".join(wrong)
    assert not servers.reached, (
        "answering these reached for something outside the test (list the route in SKIP in tests/test_routes.py, "
        "with why):\n  " + "\n  ".join(servers.reached))
    asked = {(a.server, a.path) for a in asks if a.method == "GET"}
    stale = sorted((set(SKIP) | set(NEEDS) | set(ASK)) - asked)
    assert not stale, f"SKIP, NEEDS or ASK in tests/test_routes.py list GETs no page sends any more: {stale}"
    # (and it really went round: 54 of the pages' 63 GETs today, 53 where scikit-learn isn't installed)
    assert {("label", "/api/summary"), ("label", "/thumb/"), ("tracker", "/api/people"), ("shell", "/status")} <= sent


def _installed(library):
    import importlib.util
    try:
        return importlib.util.find_spec(library) is not None
    except (ImportError, ValueError):
        return False


def test_every_page_file_is_read_by_this_test(pages):
    """A new page (or a script no page loads) would be outside this test without anyone knowing: every .html and
    .js file beside the programs must be one a server sends, reached from the pages in ENTRY."""
    plain = lambda text: text.replace("\r\n", "\n")
    sent = {plain(text) for files in pages.values() for text in files.values()}
    unread = [f.name for f in sorted(HERE.glob("*.html")) + sorted(HERE.glob("*.js"))
              if plain(f.read_text("utf-8")) not in sent
              and plain(f.read_text("utf-8").replace("{{VERSION}}", app.VERSION)) not in sent]
    assert not unread, (f"{unread}: no page in ENTRY (tests/test_routes.py) loads these. Add the new page's address "
                        f"to ENTRY for the server that sends it.")
    assert {"/label_ui.js", "/page_store.js"} <= set(pages["label"])
    assert {"/tracker_ui.js", "/cameras_ui.js", "/signals.js"} <= set(pages["tracker"])


# ---------------------------------------------------------------------------------------------- the tripwire, tripped
def test_the_reader_finds_a_request_however_it_is_written():
    """asks_in on a made-up script with each way the page scripts write a request (and what is no request)."""
    script = '''
        const r = await get("/api/summary");
        await post("/api/act", { faces });                              // "/api/in-a-comment"
        const thumbUrl = i => `/thumb/${i}.jpg?f=${encodeURIComponent(FIDS[i] || "")}`;
        fetch(`/api/capture?${new URLSearchParams({ a: JSON.stringify(b) })}`,
              { method: "POST", body: blob });
        fetch("/open?what=" + which, { headers: { "X-Face-Tool": "1" } }).then(drop);
        await alertApi("/api/alerts"); await alertApi("/api/alerts/state", body);
        x.open("GET", "/api/settings?app=" + encodeURIComponent(name), false);
        if (u.startsWith("/photo/")) keep(u);
        html = `<img src="/api/picture/${i}"><svg><path d="M0 0"/></svg>` + a.split("/").join(" / ");
        frame.src = new URL("cameras", s.url).href;
    '''
    asks, relative, unreadable = asks_in(script)
    assert [a[1:] for a in asks] == [
        ("GET", "/api/summary", False), ("POST", "/api/act", False), ("GET", "/thumb/", True),
        ("POST", "/api/capture", False), ("GET", "/open", False), ("GET", "/api/alerts", False),
        ("POST", "/api/alerts/state", False), ("GET", "/api/settings", False), ("GET", "/api/picture/", True)]
    assert [a[0] for a in asks] == [2, 3, 4, 5, 7, 8, 8, 9, 11]                    # their lines
    assert relative == [(12, "cameras")] and not unreadable
    # what it can't read is said, not passed over
    _, _, unreadable = asks_in('load("/api/x"); fetch(base + "/api/y"); fetch(u); fetch(p, { method: "POST" });')
    assert len(unreadable) == 2 and "how does load() send it" in unreadable[0] and "base +" in unreadable[1]


def test_the_reader_finds_a_route_however_it_is_written():
    """routes_in on a made-up handler: compared, looked up, started with; and what is no route."""
    source = '''
PAGES = {"/": "page.html", "/page.js": "page.js"}
POST_ROUTES = {"/api/save": save}
def handler():
    acts = ("/update", "/open")
    class Handler(Base):
        def _get(self):
            if self.refused(write=path in acts, own=path.startswith(localguard.OWN_ONLY)):
                return
            if path in PAGES: ...
            if path == "/api/summary": ...
            if path.startswith("/thumb/"): ...
            if path not in ("/status",) + acts: ...
            if kind != "label" or name.endswith("/restore") or sep == "x/y": ...
        def _post(self):
            if path in ("/api/act", "/api/undo"): ...
            match path:
                case "/api/rename": ...
def helper(path):
    return path == "/either"
TABLE = {("GET", "/api/list"): listing, ("POST", "/api/list"): add}
'''
    assert routes_in(source) == {
        Route("GET", "/", False), Route("GET", "/page.js", False), Route("POST", "/api/save", False),
        Route("GET", "/update", False), Route("GET", "/open", False), Route("GET", "/api/summary", False),
        Route("GET", "/thumb/", True), Route("GET", "/status", False), Route("POST", "/api/act", False),
        Route("POST", "/api/undo", False), Route("POST", "/api/rename", False), Route(None, "/either", False),
        Route("GET", "/api/list", False), Route("POST", "/api/list", False)}


def test_a_route_that_goes_missing_or_has_no_caller_is_noticed(pages):
    """The tripwire itself, tripped on changed copies of the labeling tool's code (whichever files ROUTE_FILES
    names for it): a route renamed (the page still asks for the old one, the new one has no caller), a prefix
    route renamed, a route for the other kind of request, the routes moved to a module this test wasn't told
    about, and the route that is being removed, gone."""
    asks, _ = page_asks(pages)
    files = {f: (HERE / f).read_text("utf-8") for f in ROUTE_FILES["label"]}
    changed = lambda old, new: server_routes({f: text.replace(old, new) for f, text in files.items()})
    told = lambda found: {say(a).split(" asks ")[1] for a in found}
    for text in ('"/api/undo"', '"/api/describe/"'):
        assert any(text in t for t in files.values()), f"{text} isn't in {sorted(files)} any more: use another route here"

    routes = changed('"/api/undo"', '"/api/undone"')
    missing = unanswered(asks, routes)
    assert told(missing) == {"POST /api/undo"} and {(a.server, a.page) for a in missing} == {("label", "/label_ui.js")}
    assert [(s, r.path) for s, r in unasked(asks, routes)] == [("label", "/api/undone")]

    # a prefix route: the page asks for /api/describe/<face>; /api/describe/status has a route of its own
    assert told(unanswered(asks, changed('"/api/describe/"', '"/api/described/"'))) == {"GET /api/describe/..."}

    # a route answers only the kind of request its handler is for, and a fixed route no address with a changing part
    get = Ask("label", "/label_ui.js", 1, "GET", "/api/checks", False)
    assert answers(Route("GET", "/api/checks", False), get) and not answers(Route("POST", "/api/checks", False), get)
    either = Route(None, "/api/checks", False)                  # the code around it doesn't say which: both
    assert answers(either, get) and answers(either, get._replace(method="POST"))
    assert not answers(Route("GET", "/api/checks", False), get._replace(path="/api/checks/", dynamic=True))
    assert answers(Route("GET", "/api/checks/", True), get._replace(path="/api/checks/", dynamic=True))

    # the labeling tool's routes gone from the files this test reads (moved, and ROUTE_FILES not told)
    lost = unanswered(asks, server_routes({f: "" for f in files if f != "localguard.py"}))
    assert {"/", "/label_ui.js", "/api/summary", "/api/act", "/thumb/", "/api/db/passphrase/approve"} <= {a.path for a in lost}
    assert {a.server for a in lost} == {"label"}

    # as the code is: nothing to say
    routes = server_routes()
    assert not unanswered(asks, routes) and not unasked(asks, routes) and not stale_callers(routes)
    # ... but any other entry of OTHER_CALLERS whose route went is to be taken out
    assert ("label", "/index.html") in stale_callers(changed('"/index.html"', '"/index.htm"'))
