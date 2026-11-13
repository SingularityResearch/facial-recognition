"""
describe.py - the labeling tool's "Describe photo": a short summary of a photo from a vision-language model
running on this computer, to help work out the occasion and who might be there.

The model runs in LM Studio (http://127.0.0.1:1234, OpenAI-style API), by default on this computer. The
Settings tab can point it at another address, also on another device (then each photo described is sent there),
and at any server with the same API (Ollama, llama.cpp, vLLM, Jan); FACE_LLM_URL sets the default address. The
choices are kept in %APPDATA%\\FaceLabels\\describe.json: which describer to use, the server, a model (or automatic)
and an optional API key. Automatic: with LM Studio any loaded vision model ("vlm"); a loaded one is preferred,
else LM Studio loads the first one it lists (about 40 s the first time); other servers don't say which of their
models read pictures, so their first model is used unless one is picked. Reasoning is switched off
(reasoning_effort "none"): with it, Qwen 3.6 35B-A3B spent ~350 of 400 tokens thinking (6.6 s a photo);
without it, 3.3 s for the same description.

Describe all can also share the photos out among the other LM Studio servers on the local network that have the
same vision model (Settings: "spread", off by default, as it sends photos to those computers): one photo at a time
to each server, each taking the next as it finishes, so a faster computer takes more (one server answering 3 at
once slowed to 45 s a photo: see label_tool's describe_all). A server that fails twice in a row is left out and
its photo goes back to the others; the configured server keeps going to the end.

When LM Studio isn't there (not running, or no vision model in it), the built-in Florence-2 model (florence.py,
downloaded on request, 0.8 GB) describes instead: plainer and less accurate, about 3.4 s a photo on the CPU.

The model is told not to guess who anyone is: names come from the face recognizer and from you.
Descriptions are saved in %APPDATA%\\FaceLabels\\descriptions.json (outside the project folder: these describe
personal photos in words), one per photo (or video frame), so each is described once.
"""
import base64
import ipaddress
import json
import os
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import urlparse

import florence
import paths

URL = os.environ.get("FACE_LLM_URL", "http://127.0.0.1:1234").rstrip("/")    # the default server
HOME = paths.HOME                # %APPDATA%\FaceLabels
STORE = HOME / "descriptions.json"
CONFIG = HOME / "describe.json"  # the Settings tab's choices: {"use", "url", "model", "key", "spread"}
USES = ("auto", "server", "builtin")   # server if it answers, else built-in / the server only / the built-in model only
TIMEOUT = 300                    # the first call may wait for LM Studio to load the model
PROMPT = ("Describe this photo for someone sorting a family photo library, in 2-3 short sentences: the setting "
          "and occasion, how many people there are and what they're doing. Then, on a new line starting 'Text:', "
          "any readable text (signs, shirts, cakes, captions, dates), or 'Text: none'. "
          "Don't guess who anyone is or name anyone.")
_lock = threading.Lock()
_cfg_lock = threading.Lock()     # describe.json: on Windows, replacing a file fails while it's being read
_saved = None


class Unavailable(Exception):
    """No local vision model to ask (LM Studio not running, or no vision model in it)."""


# --------------------------------------------------------------------------- settings
def config():
    """{"use", "url", "model", "key", "spread"} with the defaults filled in. spread: Describe all also uses the
    other LM Studio servers on the local network that have the same model (off unless chosen in Settings)."""
    try:
        with _cfg_lock:
            c = json.loads(CONFIG.read_text("utf-8"))
    except (OSError, ValueError):
        c = {}
    stored = c.get("key") if isinstance(c.get("key"), str) else ""
    try:
        key = paths.reveal(stored)                 # kept protected (paths.protect); an older plain one as it is
    except (OSError, ValueError) as e:
        print(f"Describe photo: the saved API key can't be read here ({e})")
        key = ""
    c = {"use": c.get("use") if c.get("use") in USES else "auto", "url": c.get("url") or URL,
         "model": c.get("model") or "", "key": key, "spread": c.get("spread") is True}
    if key and not paths.is_protected(stored):     # saved plain by an older version: protect it now
        try:
            _write_config(c)
        except OSError:
            pass
    return c


def _write_config(c):
    """describe.json, with the API key protected for this user."""
    with _cfg_lock:
        paths.write_private(CONFIG, json.dumps(dict(c, key=paths.protect(c["key"]) if c["key"] else ""), indent=1))


def clean_url(url):
    """'host:port', 'http://host:port/v1/' ... -> 'http://host:port'. Raises ValueError."""
    url = (url or "").strip()
    if url and "://" not in url:
        url = "http://" + url
    url = url.rstrip("/")
    if url.endswith("/v1"):
        url = url[:-3]
    u = urlparse(url)
    if u.scheme not in ("http", "https") or not u.hostname:
        raise ValueError("That isn't a server address: use one like http://127.0.0.1:1234 or http://192.168.1.20:1234")
    try:
        _ = u.port                              # reading it checks it (number, 0-65535)
    except ValueError:
        raise ValueError("The port in that address isn't a number") from None
    return url


def set_config(**kw):
    """Change some of the settings (use, url, model, key); an empty url means the default."""
    c = config()
    if "use" in kw:
        if kw["use"] not in USES:
            raise ValueError("Unknown choice")
        c["use"] = kw["use"]
    if "url" in kw:
        url = clean_url(kw["url"]) if (kw["url"] or "").strip() else URL
        if url != c["url"] and "key" not in kw:
            c["key"] = ""                       # a key belongs to its server: never send it to another one
        c["url"] = url
    if "model" in kw:
        c["model"] = str(kw["model"] or "").strip()[:200]
    if "key" in kw:
        c["key"] = str(kw["key"] or "").strip()[:500]
    if "spread" in kw:
        c["spread"] = kw["spread"] is True
    _write_config(c)
    return c


def where(url):
    """'this computer', 'your network' or 'the internet' for a server address, and whether pictures sent
    there would travel unencrypted over the internet."""
    u = urlparse(url)
    host = u.hostname or ""
    if host in ("localhost", "127.0.0.1", "::1"):
        return "this computer", False
    try:
        ip = ipaddress.ip_address(host)
        if ip.is_loopback:
            return "this computer", False
        if ip.is_private or ip.is_link_local:
            return "your network", False
    except ValueError:
        if host.endswith((".local", ".lan", ".home", ".internal")) or "." not in host:
            return "your network", False
    return "the internet", u.scheme != "https"


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """A describe server's answer is taken as it is: a redirect isn't followed (it would carry the API key
    and the photo to another address)."""
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


_opener = urllib.request.build_opener(_NoRedirect)
MAX_ANSWER = 4_000_000                   # bytes read from a describe server's answer
MAX_TEXT = 4000                          # characters of a description kept


def _open(req, timeout):
    """JSON from a describe server: no redirects, at most MAX_ANSWER bytes."""
    with _opener.open(req, timeout=timeout) as r:
        data = r.read(MAX_ANSWER + 1)
    if len(data) > MAX_ANSWER:
        raise ValueError("The describe server's answer was too large")
    return json.loads(data)


def _get(path, timeout=5, url=None, key=None):
    """GET from the saved server with its key, or from url with only the key given for it."""
    c = config()
    if url is None:
        url, key = c["url"], c["key"]
    req = urllib.request.Request(url + path)
    if key:
        req.add_header("Authorization", "Bearer " + key)
    return _open(req, timeout)


def server_models(url=None, key=None):
    """The models a server offers: ([(id, reads pictures: True/None), ...], loaded ids). LM Studio says which
    are vision models and which are loaded; other OpenAI-style servers only list ids. Raises Unavailable."""
    no_answer = (f"No describe server answers at {url or config()['url']} (LM Studio: start it and turn on "
                 f"its local server; on another device, also 'Serve on local network')")
    try:
        ms = _get("/api/v0/models", 3, url, key).get("data", [])      # LM Studio's own list
        return [(m["id"], m.get("type") == "vlm") for m in ms if m.get("type") in ("vlm", "llm")], \
               [m["id"] for m in ms if m.get("state") == "loaded"]
    except (urllib.error.HTTPError, ValueError, KeyError, AttributeError, TypeError):
        pass                                     # it answered, but isn't LM Studio: ask the standard way
    except OSError as e:                         # nothing there (refused, timed out, unknown name): don't wait twice
        raise Unavailable(no_answer) from e
    try:
        ms = _get("/v1/models", 3, url, key).get("data", [])
        return [(m["id"], None) for m in ms if isinstance(m, dict) and m.get("id")], []
    except urllib.error.HTTPError as e:
        raise Unavailable(f"The describe server at {url or config()['url']} refused ({e.code}"
                          f"{': check the API key' if e.code in (401, 403) else ''})") from e
    except (OSError, ValueError, AttributeError) as e:
        raise Unavailable(no_answer) from e


def model(found=None):
    """The vision model to use, as chosen in Settings: the server's (the one picked, or automatic), else the
    built-in one if it's downloaded. Raises Unavailable."""
    use = config()["use"]
    if use == "builtin":
        if florence.ready():
            return florence.NAME
        raise Unavailable(f"The built-in model isn't downloaded yet ({florence.SIZE / 1e9:.1f} GB)")
    try:
        return lmstudio_model(found)
    except Unavailable as e:
        if use == "auto" and florence.ready():
            return florence.NAME
        if use == "server":
            raise
        raise Unavailable(f"{e}, or download the built-in model ({florence.SIZE / 1e9:.1f} GB, plainer descriptions)") from e


def lmstudio_model(found=None):
    """The server's model to use: the one picked in Settings, else a loaded vision model, else the first
    vision model (LM Studio), else the server's first model (other servers). Raises Unavailable."""
    if isinstance(found, Unavailable):          # status() already asked, and nothing answered
        raise found
    ms, loaded = found or server_models()
    pick = config()["model"]
    if pick:
        if ms and pick not in [m for m, _ in ms]:
            raise Unavailable(f"The describe server has no model called {pick}: pick another in Settings")
        return pick
    vlm = [m for m, v in ms if v] if any(v is not None for _, v in ms) else [m for m, _ in ms]
    if not vlm:
        raise Unavailable("The describe server has no vision model: download one (for example Qwen 3.6) and try again")
    return ([m for m in vlm if m in loaded] or vlm)[0]


def status():
    """{"ok": bool, "model" or "why", "builtin", "config", "models"} for the page (never the API key)."""
    b, c = florence.status(), config()
    place, open_net = where(c["url"])
    out = {"builtin": b, "config": {"use": c["use"], "url": c["url"], "model": c["model"], "key": bool(c["key"]),
                                   "default_url": URL, "where": place, "unencrypted": open_net}}
    found, out["models"] = None, []
    if c["use"] != "builtin":                    # ask the server once, for the list and for the model to use
        try:
            found = server_models()
            out["models"] = [m for m, v in found[0] if v is not False]
        except Unavailable as e:
            found = e
    try:
        out.update(ok=True, model=model(found))
    except Unavailable as e:
        out.update(ok=False, why=str(e))
    out["config"]["spread"] = c["spread"]
    out["helpers"] = helpers(out.get("model")) if out["ok"] else []
    return out


def test(url, key=None):
    """For Settings' Test button: what the server at url offers, or why it doesn't answer."""
    url, c = clean_url(url), config()
    ms, loaded = server_models(url, key or (c["key"] if url == c["url"] else None))   # the saved key: its own server only
    vision = [m for m, v in ms if v]
    if any(v is not None for _, v in ms):       # LM Studio
        return f"LM Studio answers: {len(vision)} vision model(s)" + (f": {', '.join(vision[:5])}" if vision else
               ". Download one (for example Qwen 3.6) in LM Studio")
    return f"The server answers: {len(ms)} model(s){': ' + ', '.join(m for m, _ in ms[:5]) if ms else ''}"


def helpers(name):
    """The other LM Studio servers Describe all may also send photos to: [{"host", "url"}], only with "spread"
    on, a server model (not the built-in one) and on this computer's local network (hardware.lmstudio_network:
    found by port and mDNS on its own networks). Each must list the same model as a vision model, so every
    description comes from the same model. The configured server itself isn't one of them. No API key goes to them."""
    c = config()
    if not c["spread"] or c["use"] == "builtin" or not name or name == florence.NAME:
        return []
    import hardware
    main_here = where(c["url"])[0] == "this computer"
    out = []
    for e in hardware.lmstudio_network():
        if c["url"] in e["urls"] or (main_here and e["this_computer"]):
            continue
        if where(e["urls"][0])[0] == "the internet" and not hardware.distributed():
            continue                             # beyond the local network only as a "Fully distributed system"
        if not any(m["id"] == name and m["vision"] for m in e["models"]):
            continue
        # the address that connects first (a computer on Wi-Fi and wired answers at both)
        best = min(e["urls"], key=_connect_secs)
        out.append({"host": e["host"], "url": best})
    return out


def _connect_secs(url):
    """Seconds to open a connection to url's host and port (9 if it doesn't)."""
    import socket
    u = urlparse(url)
    t = time.perf_counter()
    try:
        with socket.create_connection((u.hostname, u.port or 80), timeout=0.5):
            return time.perf_counter() - t
    except OSError:
        return 9.0


def ask(jpeg, name=None, url=None):
    """Describe one JPEG with the local model, or with the server at url (one of helpers(): no API key is sent
    there). Returns {"text", "model", "secs"}."""
    name = name or model()
    if name == florence.NAME:
        try:
            return florence.describe(jpeg)
        except OSError as e:
            raise Unavailable(str(e)) from e
    body = {"model": name, "temperature": 0.2, "max_tokens": 400, "reasoning_effort": "none",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": PROMPT},
                {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64," + base64.b64encode(jpeg).decode()}}]}]}
    t, c = time.time(), config()
    headers = {"Content-Type": "application/json"}
    key = c["key"] if url is None or url == c["url"] else ""      # a key belongs to its own server only
    if key:
        headers["Authorization"] = "Bearer " + key
    for attempt in (0, 1):
        req = urllib.request.Request((url or c["url"]) + "/v1/chat/completions", data=json.dumps(body).encode(), headers=headers)
        try:
            out = _open(req, TIMEOUT)
            break
        except urllib.error.HTTPError as e:
            why = e.read()[:200].decode("utf-8", "replace")
            if attempt == 0 and e.code in (400, 422) and "reasoning" in why:   # a server that doesn't know it
                body.pop("reasoning_effort")
                continue
            raise Unavailable(f"The describe server couldn't describe it ({e.code}: {why})") from e
        except OSError as e:
            raise Unavailable(f"The describe server didn't answer: {e}") from e
    try:
        text = out["choices"][0]["message"].get("content") or ""
    except (KeyError, IndexError, TypeError, AttributeError):
        raise Unavailable("The describe server's answer wasn't understood") from None
    if not isinstance(text, str):
        raise Unavailable("The describe server's answer wasn't text")
    text = text.strip()[:MAX_TEXT]
    if not text:
        raise Unavailable("The model gave an empty answer")
    return {"text": text, "model": name, "secs": round(time.time() - t, 1)}


# --------------------------------------------------------------------------- saved descriptions
def _load():
    global _saved
    if _saved is None:
        s = paths.read_json(STORE, {})             # OSError if it's there and can't be read: never taken as empty
        _saved = s if isinstance(s, dict) else {}
    return _saved


def saved(key):
    with _lock:
        return _load().get(key)


def save(key, d, write=True):
    """Keep a description; write=False leaves writing the file to flush() (the describe-all pass writes
    every 25 photos, not after each: the file grows to several MB)."""
    with _lock:
        s = _load()
        s[key] = dict(d, when=paths.utc_now())
        if write:
            _write(s)
        return s[key]


def flush():
    with _lock:
        _write(_load())


def _write(s):
    paths.atomic_write(STORE, json.dumps(s, ensure_ascii=False, indent=0))
    import face_id as F
    for hook in F.SAVED_HOOKS:                   # the shared database's sync sends it within seconds
        hook()


def count():
    with _lock:
        return len(_load())


def find(match):
    """{photo key: description} of the saved descriptions match(description) is true for (persondata.py)."""
    with _lock:
        return {k: dict(d) for k, d in _load().items() if match(d)}


def drop(match):
    """Delete the saved descriptions match(description) is true for (forgetting a person); returns how many."""
    with _lock:
        s = _load()
        gone = [k for k, d in s.items() if match(d)]
        for k in gone:
            del s[k]
        if gone:
            _write(s)
        return len(gone)
