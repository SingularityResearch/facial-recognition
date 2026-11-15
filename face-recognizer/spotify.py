"""
spotify.py - the owner's Spotify library, read with their permission, for the music picture (music.py).

Only when the owner connects it (Settings, or `python spotify.py connect <client id>`): the app never contacts
Spotify on its own. What it reads: the saved ("liked") songs of the account that agreed, and the genres Spotify
lists for their artists. Read-only (scope user-library-read): nothing in the account can be changed.

How the permission works (Spotify's "authorization code with PKCE", no client secret):
  1. The owner makes an app key of their own at developer.spotify.com (a client id) with the redirect address
     redirect_uri() below. The id is not a secret.
  2. begin() gives the address of Spotify's own page, where the owner signs in and agrees. Spotify then sends the
     browser to redirect_uri() with a one-time code.
  3. wait_for_code() listens on 127.0.0.1 for that one request (checked against the state it gave out), and
     finish() trades the code for the tokens.
The tokens are kept in HOME/spotify.json, protected (paths.protect): never in the repo, never in data/.

What reaches the network: accounts.spotify.com (the permission, the tokens) and api.spotify.com (the library),
over HTTPS, each only on the owner's click.
"""
import base64
import hashlib
import hmac
import http.server
import json
import os
import secrets
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

import paths

CONF = paths.HOME / "spotify.json"
AUTH = "https://accounts.spotify.com/authorize"
TOKEN = "https://accounts.spotify.com/api/token"
API = "https://api.spotify.com/v1"
SCOPE = "user-library-read"
TIMEOUT = 20                     # seconds for one request
MAX_ANSWER = 8_000_000           # bytes of one answer
PAGE = 50                        # saved songs in one request (Spotify's most)
MAX_SONGS = 20_000               # a library larger than this is read up to here
_lock = threading.Lock()


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Spotify's two addresses answer themselves. A redirect is never followed: urllib would carry the request's
    token along to wherever it pointed."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url, code, f"redirected to {newurl[:100]} (not followed)", headers, fp)


_opener = urllib.request.build_opener(_NoRedirect)


class NotConnected(Exception):
    """Spotify isn't connected on this device (or the permission was taken back)."""


def callback_port():
    """Where the one-time code arrives: the app's tab page port + 4 (8768), beside the peer service's + 3."""
    base = os.environ.get("FACE_PORTS") or ""
    return (int(base) if base.isdigit() else 8764) + 4


def redirect_uri():
    return f"http://127.0.0.1:{callback_port()}/callback"


# --- what is kept ---------------------------------------------------------------------------------------------------

def _load():
    return paths.read_json(CONF, {}) if CONF.exists() else {}


def _save(c):
    paths.write_private(CONF, json.dumps(c, indent=1))


def status():
    """{connected, client_id, since}: what Settings shows. Never a token."""
    c = _load()
    return {"connected": bool(c.get("refresh")), "client_id": c.get("client_id") or "", "since": c.get("since")}


def disconnect():
    """Forget the tokens here (the permission itself is taken back in the Spotify account: Apps)."""
    c = _load()
    _save({"client_id": c.get("client_id") or ""})


# --- asking for the permission ----------------------------------------------------------------------------------------

def check_client_id(client_id):
    client_id = (client_id or "").strip()
    if not (len(client_id) == 32 and all(ch in "0123456789abcdef" for ch in client_id.lower())):
        raise ValueError("a Spotify client id is 32 letters and digits (Settings of your app at developer.spotify.com)")
    return client_id


def begin(client_id):
    """(address of Spotify's page to open, verifier, state) for one request for permission."""
    client_id = check_client_id(client_id)
    verifier = secrets.token_urlsafe(64)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    state = secrets.token_urlsafe(24)
    q = urllib.parse.urlencode({"client_id": client_id, "response_type": "code", "redirect_uri": redirect_uri(),
                                "code_challenge_method": "S256", "code_challenge": challenge, "scope": SCOPE,
                                "state": state})
    return f"{AUTH}?{q}", verifier, state


def wait_for_code(state, timeout=300, port=None):
    """Listen on 127.0.0.1 for Spotify's one answer to the request `state` belongs to; the code, or raise OSError
    (refused, or nobody answered in time). Anything else that arrives is answered 404 and ignored."""
    got = {}

    class Once(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            u = urllib.parse.urlsplit(self.path)
            q = dict(urllib.parse.parse_qsl(u.query))
            mine = u.path == "/callback" and hmac.compare_digest(q.get("state", ""), state)
            if mine:
                got.update(code=q.get("code"), error=q.get("error"))
            text = ("Spotify is connected. You can close this tab." if mine and q.get("code") else
                    "Spotify was not connected." if mine else "Not found.")
            body = f"<!doctype html><meta charset=utf-8><title>Face Recognizer</title><p>{text}</p>".encode()
            self.send_response(200 if mine else 404)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

    srv = http.server.HTTPServer(("127.0.0.1", callback_port() if port is None else port), Once)
    srv.timeout = 1
    end = time.time() + timeout
    try:
        while not got and time.time() < end:
            srv.handle_request()
    finally:
        srv.server_close()
    if got.get("code"):
        return got["code"]
    raise OSError("Spotify said no: " + str(got["error"])[:100] if got.get("error") else "nobody agreed in time")


def _post_token(fields):
    req = urllib.request.Request(TOKEN, data=urllib.parse.urlencode(fields).encode(),
                                 headers={"Content-Type": "application/x-www-form-urlencoded"})
    try:
        with _opener.open(req, timeout=TIMEOUT) as r:
            return json.loads(r.read(MAX_ANSWER))
    except urllib.error.HTTPError as e:
        why = e.read(2000).decode("utf-8", "replace")
        try:
            why = json.loads(why).get("error_description") or json.loads(why).get("error") or why
        except ValueError:
            pass
        raise OSError(f"Spotify refused ({e.code}): {str(why)[:200]}") from None


def finish(code, verifier, client_id):
    """Trade the one-time code for the tokens and keep them (protected)."""
    client_id = check_client_id(client_id)
    t = _post_token({"grant_type": "authorization_code", "code": code, "redirect_uri": redirect_uri(),
                     "client_id": client_id, "code_verifier": verifier})
    if not t.get("refresh_token") or not t.get("access_token"):
        raise OSError("Spotify's answer had no tokens")
    with _lock:
        _save({"client_id": client_id, "refresh": paths.protect(t["refresh_token"]),
               "access": paths.protect(t["access_token"]), "expires": time.time() + int(t.get("expires_in", 3600)) - 60,
               "since": paths.utc_now()})
    return status()


def _access():
    """A token for one request, renewed when the last one is used up."""
    with _lock:
        c = _load()
        if not c.get("refresh"):
            raise NotConnected("Spotify isn't connected on this device")
        if c.get("access") and time.time() < float(c.get("expires") or 0):
            return paths.unprotect(c["access"])
        try:
            t = _post_token({"grant_type": "refresh_token", "refresh_token": paths.unprotect(c["refresh"]),
                             "client_id": c["client_id"]})
        except OSError as e:
            if "invalid_grant" in str(e) or "revoked" in str(e).lower():
                _save({"client_id": c.get("client_id") or ""})
                raise NotConnected("Spotify's permission was taken back: connect again") from None
            raise
        c["access"] = paths.protect(t["access_token"])
        c["expires"] = time.time() + int(t.get("expires_in", 3600)) - 60
        if t.get("refresh_token"):                           # Spotify may hand out a new one
            c["refresh"] = paths.protect(t["refresh_token"])
        _save(c)
        return t["access_token"]


# --- reading the library ------------------------------------------------------------------------------------------

def _get(path, **params):
    url = f"{API}{path}" + ("?" + urllib.parse.urlencode(params) if params else "")
    for attempt in range(4):
        req = urllib.request.Request(url, headers={"Authorization": "Bearer " + _access()})
        try:
            with _opener.open(req, timeout=TIMEOUT) as r:
                return json.loads(r.read(MAX_ANSWER))
        except urllib.error.HTTPError as e:
            if e.code == 429 and attempt < 3:                # asked too fast: wait as long as Spotify says
                time.sleep(min(30, int(e.headers.get("Retry-After") or 2)) + 0.5)
                continue
            if e.code == 401 and attempt == 0:               # the token ran out between the check and the request
                with _lock:
                    c = _load()
                    c["expires"] = 0
                    _save(c)
                continue
            raise OSError(f"Spotify answered {e.code} for {path}: {e.read(300).decode('utf-8', 'replace')[:200]}") from None
    raise OSError("Spotify keeps asking to wait")


def _song(item):
    """One saved song as the picture needs it: short text and numbers only."""
    t = item.get("track") or {}
    if not isinstance(t, dict) or not t.get("id") or t.get("type", "track") != "track":
        return None
    album = t.get("album") or {}
    text = lambda v, n=200: v[:n] if isinstance(v, str) else ""            # noqa: E731
    return {"id": text(t["id"], 40), "name": text(t.get("name")),
            "artists": [{"id": text(a.get("id"), 40), "name": text(a.get("name"))}
                        for a in (t.get("artists") or [])[:8] if isinstance(a, dict)],
            "album": text(album.get("name")), "released": text(album.get("release_date"), 10),
            "added": text(item.get("added_at"), 25), "ms": int(t.get("duration_ms") or 0),
            "explicit": bool(t.get("explicit")), "popularity": t.get("popularity") if isinstance(t.get("popularity"), int) else None}


def liked(progress=None):
    """Every saved song of the connected account, newest first: [{id, name, artists, album, released, added, ms,
    explicit, popularity}]. progress(done, total) is called as the pages come in."""
    out, offset, total = [], 0, None
    while total is None or (offset < total and len(out) < MAX_SONGS):
        page = _get("/me/tracks", limit=PAGE, offset=offset)
        total = int(page.get("total") or 0)
        items = page.get("items") or []
        if not items:
            break
        out += [s for s in map(_song, items) if s]
        offset += len(items)
        if progress:
            progress(min(offset, total), total)
    return out


def genres(artist_ids, progress=None):
    """{artist id: [genres]} as Spotify lists them (many artists have none). Empty when Spotify no longer gives
    them to this kind of key: the picture then goes without genres."""
    ids = sorted({a for a in artist_ids if a})
    out = {}
    for k in range(0, len(ids), 50):
        try:
            page = _get("/artists", ids=",".join(ids[k:k + 50]))
        except OSError:
            if k == 0:
                return {}                                    # not allowed, or gone: no genres at all
            raise
        for a in page.get("artists") or []:
            if isinstance(a, dict) and a.get("id"):
                out[a["id"]] = [g[:60] for g in (a.get("genres") or []) if isinstance(g, str)][:12]
        if progress:
            progress(min(k + 50, len(ids)), len(ids))
    return out


# --- command line -----------------------------------------------------------------------------------------------

def connect(client_id, open_browser=True, say=print):
    """Ask for the permission and wait for the owner to agree in their browser."""
    url, verifier, state = begin(client_id)
    say("Opening Spotify's page in your browser: sign in there and agree.")
    say(f"(If it doesn't open, go to:\n{url}\n)")
    if open_browser:
        import webbrowser
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    code = wait_for_code(state)
    return finish(code, verifier, client_id)


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["connect"] and len(args) == 2:
        print(connect(args[1]))
    elif args == ["status"]:
        print(status())
    elif args == ["disconnect"]:
        disconnect()
        print(status())
    else:
        sys.exit("python spotify.py connect <client id> | status | disconnect   (redirect address of the key: "
                 + redirect_uri() + ")")
