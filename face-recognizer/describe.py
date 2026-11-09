"""
describe.py - the labeling tool's "Describe photo": a short summary of a photo from a vision-language model
running on this computer, to help work out the occasion and who might be there.

The model runs in LM Studio (http://127.0.0.1:1234, OpenAI-style API; another address: FACE_LLM_URL), on this
computer only: nothing is sent anywhere else. Any loaded vision model ("vlm") works; a loaded one is preferred,
else LM Studio loads the first one it lists (about 40 s the first time). Reasoning is switched off
(reasoning_effort "none"): with it, Qwen 3.6 35B-A3B spent ~350 of 400 tokens thinking (6.6 s a photo);
without it, 3.3 s for the same description.

When LM Studio isn't there (not running, or no vision model in it), the built-in Florence-2 model (florence.py,
downloaded on request, 0.8 GB) describes instead: plainer and less accurate, about 3.4 s a photo on the CPU.

The model is told not to guess who anyone is: names come from the face recognizer and from you.
Descriptions are saved in %APPDATA%\\FaceLabels\\descriptions.json (outside the project: data\\ is published,
and these describe personal photos in words), one per photo (or video frame), so each is described once.
"""
import base64
import json
import os
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

import florence

URL = os.environ.get("FACE_LLM_URL", "http://127.0.0.1:1234").rstrip("/")
HOME = Path(os.environ.get("APPDATA", Path.home())) / "FaceLabels"
STORE = HOME / "descriptions.json"
TIMEOUT = 300                    # the first call may wait for LM Studio to load the model
PROMPT = ("Describe this photo for someone sorting a family photo library, in 2-3 short sentences: the setting "
          "and occasion, how many people there are and what they're doing. Then, on a new line starting 'Text:', "
          "any readable text (signs, shirts, cakes, captions, dates), or 'Text: none'. "
          "Don't guess who anyone is or name anyone.")
_lock = threading.Lock()
_saved = None


class Unavailable(Exception):
    """No local vision model to ask (LM Studio not running, or no vision model in it)."""


def _get(path, timeout=5):
    with urllib.request.urlopen(URL + path, timeout=timeout) as r:
        return json.load(r)


def model():
    """The vision model to use: LM Studio's (a loaded one if any, else the first it lists), else the built-in
    one if it's downloaded. Raises Unavailable."""
    try:
        return lmstudio_model()
    except Unavailable as e:
        if florence.ready():
            return florence.NAME
        raise Unavailable(f"{e}, or download the built-in model ({florence.SIZE / 1e9:.1f} GB, plainer descriptions)") from e


def lmstudio_model():
    try:
        ms = _get("/api/v0/models").get("data", [])
    except (OSError, ValueError) as e:
        raise Unavailable("LM Studio isn't running (start it and turn on its local server, port 1234)") from e
    vlm = [m for m in ms if m.get("type") == "vlm"]
    if not vlm:
        raise Unavailable("LM Studio has no vision model: download one (for example Qwen 3.6) and try again")
    loaded = [m for m in vlm if m.get("state") == "loaded"]
    return (loaded or vlm)[0]["id"]


def status():
    """{"ok": bool, "model" or "why"} for the page."""
    b = florence.status()
    try:
        return {"ok": True, "model": model(), "builtin": b}
    except Unavailable as e:
        return {"ok": False, "why": str(e), "builtin": b}


def ask(jpeg, name=None):
    """Describe one JPEG with the local model. Returns {"text", "model", "secs"}."""
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
    t = time.time()
    req = urllib.request.Request(URL + "/v1/chat/completions", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            out = json.load(r)
    except urllib.error.HTTPError as e:
        raise Unavailable(f"LM Studio couldn't describe it ({e.code}: {e.read()[:200].decode('utf-8', 'replace')})") from e
    except OSError as e:
        raise Unavailable(f"LM Studio didn't answer: {e}") from e
    text = (out["choices"][0]["message"].get("content") or "").strip()
    if not text:
        raise Unavailable("The model gave an empty answer")
    return {"text": text, "model": name, "secs": round(time.time() - t, 1)}


# --------------------------------------------------------------------------- saved descriptions
def _load():
    global _saved
    if _saved is None:
        try:
            _saved = json.loads(STORE.read_text("utf-8"))
        except (OSError, ValueError):
            _saved = {}
    return _saved


def saved(key):
    with _lock:
        return _load().get(key)


def save(key, d, write=True):
    """Keep a description; write=False leaves writing the file to flush() (the describe-all pass writes
    every 25 photos, not after each: the file grows to several MB)."""
    with _lock:
        s = _load()
        s[key] = dict(d, when=time.strftime("%Y-%m-%d %H:%M"))
        if write:
            _write(s)
        return s[key]


def flush():
    with _lock:
        _write(_load())


def _write(s):
    STORE.parent.mkdir(parents=True, exist_ok=True)
    tmp = STORE.with_suffix(".tmp")
    tmp.write_text(json.dumps(s, ensure_ascii=False, indent=0), "utf-8")
    os.replace(tmp, STORE)


def count():
    with _lock:
        return len(_load())
