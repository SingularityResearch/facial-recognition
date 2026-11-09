"""
florence.py - the built-in photo describer, used by "Describe photo" when LM Studio isn't available.

Microsoft's Florence-2 large (MIT licence), as exported to ONNX by onnx-community, 8-bit ("quantized"), run
with onnxruntime on the CPU: about 0.8 GB, downloaded on request (not with the other models) from Hugging Face,
pinned to one revision and checked by SHA-256. It writes a caption (<MORE_DETAILED_CAPTION>) and reads text
(<OCR>); the answer has the same shape as LM Studio's: a few sentences, then "Text: ...".

Measured on 18 of the library's photos (2026-11-09), against Qwen 3.6 35B-A3B in LM Studio:
- 3.4 s a photo on this computer's CPU (base fp32: 1.4 s; large fp16 without a cache: 12.5 s).
- Plain, literal captions: it sees what is there (a man in a cap, a jeep in a parking lot) but not what the
  picture is (a meme, a game screenshot, a scanned envelope), and gets some things wrong (a hurt finger as toes).
  Qwen's descriptions are clearly better; this is the fallback.
- Text: clean, large text is read well (captions, meme text, game screens); small or curved text comes out garbled.
- DirectML gave garbage for every Florence-2 export tried (base and large, fp32 and fp16), so it runs on the CPU.
  The fp16 large decoders don't load in onnxruntime 1.24 (merged: invalid graph; with-past: fixed length 16).

The tokenizer (BART's byte-level BPE) is written out below, so nothing beyond onnxruntime is needed.
"""
import hashlib
import json
import os
import re
import sys
import threading
import time
import urllib.request
from pathlib import Path

import cv2
import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = Path(sys.executable).resolve().parent if getattr(sys, "frozen", False) else HERE   # as face_id.ROOT
DIR = ROOT / "models" / "florence2"
NAME = "Florence-2 large (built-in)"
REPO = "onnx-community/Florence-2-large-ft"
REV = "04ace74913c28d7ec94af32cee5c111a10126b6f"
# file: (size in bytes, sha256)
FILES = {
    "vocab.json": (798293, "ed19656ea1707df69134c4af35c8ceda2cc9860bf2c3495026153a133670ab5e"),
    "merges.txt": (456318, "1ce1664773c50f3e0cc8842619a93edc4624525b728b188a9e0be33b7726adc5"),
    "added_tokens.json": (22410, "ba67457c577895525996e7a42d7da60a05856506ea4fd25ff9a023a64662f74b"),
    "onnx/vision_encoder_quantized.onnx": (366371802, "0dbf2172ef1dc6fbb4da0f02498ef9e3a009bf84e8f2e0d971f39cc5b7e053ce"),
    "onnx/embed_tokens_quantized.onnx": (52520480, "dab1d09ab449f1302df6e439b35ec9e93fb037134bf6bed9ffeaa93a5b35d43c"),
    "onnx/encoder_model_quantized.onnx": (152985855, "9fe30b02cc8c2247631f8f0b9bc4d6613df1c33da0afe4087abff2f940bc2fe0"),
    "onnx/decoder_model_merged_quantized.onnx": (257619824, "167e7807fb1e5b8d693138f84bc81e1368ea11833264131c1ea96eafc9d13222"),
}
SIZE = sum(s for s, _ in FILES.values())
PROMPTS = {"<MORE_DETAILED_CAPTION>": "Describe with a paragraph what is shown in the image.",
           "<OCR>": "What is the text in the image?"}
MAX_NEW = 256            # tokens per answer
NGRAM = 3                # no_repeat_ngram_size, as in the model's generation_config.json
WORD = re.compile(r"""'s|'t|'re|'ve|'m|'ll|'d| ?[^\W\d_]+| ?\d+| ?[^\s\w]+|\s+(?!\S)|\s+""")

_lock = threading.Lock()          # one description at a time (the model uses every CPU core)
_model = None
download_state = {"running": False, "done": 0, "total": SIZE, "error": None}


def ready():
    return all((DIR / f).exists() and (DIR / f).stat().st_size == s for f, (s, _) in FILES.items())


def status():
    """For the page: is the built-in model here, or being downloaded."""
    return {"name": NAME, "ready": ready(), "size_mb": round(SIZE / 1e6), **{k: download_state[k] for k in ("running", "done", "error")}}


# --------------------------------------------------------------------------- download
def download():
    """Fetch the model files that are missing (blocking; progress in download_state). Raises OSError."""
    st = download_state
    st.update(running=True, done=0, error=None)
    try:
        for name, (size, sha) in FILES.items():
            dest = DIR / name
            if dest.exists() and dest.stat().st_size == size:
                st["done"] += size
                continue
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_name(dest.name + ".download")
            url = f"https://huggingface.co/{REPO}/resolve/{REV}/{name}"
            h, start = hashlib.sha256(), st["done"]
            req = urllib.request.Request(url, headers={"User-Agent": "FaceRecognizer-model-fetch"})
            try:
                with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "wb") as out:
                    while chunk := r.read(1 << 20):
                        out.write(chunk)
                        h.update(chunk)
                        st["done"] += len(chunk)
                if h.hexdigest() != sha:
                    raise OSError(f"{name} downloaded, but it isn't the expected file (checksum differs)")
                os.replace(tmp, dest)
            except Exception:
                tmp.unlink(missing_ok=True)
                st["done"] = start
                raise
    except Exception as e:
        st["error"] = f"Couldn't download the built-in model: {e}"
        raise OSError(st["error"]) from e
    finally:
        st["running"] = False


def download_in_background():
    if download_state["running"]:
        return
    threading.Thread(target=lambda: _quiet(download), daemon=True).start()


def _quiet(f):
    try:
        f()
    except OSError:
        pass                      # the reason is in download_state["error"]


# --------------------------------------------------------------------------- tokenizer (byte-level BPE)
def _byte_chars():
    bs = list(range(33, 127)) + list(range(161, 173)) + list(range(174, 256))
    cs, n = bs[:], 0
    for b in range(256):
        if b not in bs:
            bs.append(b)
            cs.append(256 + n)
            n += 1
    return dict(zip(bs, map(chr, cs)))


class _Tokenizer:
    def __init__(self, d):
        self.vocab = json.loads((d / "vocab.json").read_text("utf-8"))
        added = json.loads((d / "added_tokens.json").read_text("utf-8"))
        self.skip = set(added.values()) | {0, 1, 2, 3}          # <s> <pad> </s> <unk> and the location tokens
        self.inv = {v: k for k, v in {**self.vocab, **added}.items()}
        merges = (d / "merges.txt").read_text("utf-8").splitlines()[1:]
        self.ranks = {tuple(m.split()): i for i, m in enumerate(merges) if m}
        self.b2c = _byte_chars()
        self.c2b = {c: b for b, c in self.b2c.items()}

    def _bpe(self, w):
        parts = list(w)
        while len(parts) > 1:
            r, i = min((self.ranks.get(p, 1e9), i) for i, p in enumerate(zip(parts, parts[1:])))
            if r == 1e9:
                break
            parts[i:i + 2] = [parts[i] + parts[i + 1]]
        return parts

    def encode(self, text):
        ids = [0]
        for w in WORD.findall(text):
            ids += [self.vocab[p] for p in self._bpe("".join(self.b2c[b] for b in w.encode("utf-8")))]
        return ids + [2]

    def decode(self, ids):
        s = "".join(self.inv.get(i, "") for i in ids if i not in self.skip)
        return bytes(self.c2b[c] for c in s if c in self.c2b).decode("utf-8", "replace").strip()


# --------------------------------------------------------------------------- the model
class _Florence:
    def __init__(self, d):
        import onnxruntime as ort
        o = ort.SessionOptions()
        o.log_severity_level = 3
        mk = lambda n: ort.InferenceSession(str(d / "onnx" / f"{n}_quantized.onnx"), o, providers=["CPUExecutionProvider"])
        self.vis, self.emb, self.enc, self.dec = (mk(n) for n in ("vision_encoder", "embed_tokens", "encoder_model", "decoder_model_merged"))
        self.tok = _Tokenizer(d)
        self.past = [i.name for i in self.dec.get_inputs() if i.name.startswith("past_key_values")]
        self.heads = next(i.shape[1] for i in self.dec.get_inputs() if i.name.startswith("past"))
        self.outs = [o.name for o in self.dec.get_outputs()]

    def features(self, bgr):
        x = cv2.resize(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB), (768, 768), interpolation=cv2.INTER_CUBIC).astype(np.float32) / 255
        x = (x - np.float32([0.485, 0.456, 0.406])) / np.float32([0.229, 0.224, 0.225])
        return self.vis.run(None, {"pixel_values": x.transpose(2, 0, 1)[None].astype(np.float32)})[0]

    def answer(self, feats, task):
        """Greedy decoding with the model's no-repeat-3-gram rule (its config asks for 3 beams; greedy was
        close in tests and 3x cheaper)."""
        ids = np.array([self.tok.encode(PROMPTS[task])], np.int64)
        e = np.concatenate([feats, self.emb.run(None, {"input_ids": ids})[0]], 1)
        mask = np.ones(e.shape[:2], np.int64)
        hid = self.enc.run(None, {"inputs_embeds": e, "attention_mask": mask})[0]
        z = np.zeros((1, self.heads, 0, 64), np.float32)
        past, out = {n: z for n in self.past}, [2]
        for step in range(MAX_NEW):
            x = self.emb.run(None, {"input_ids": np.array([out[-1:]], np.int64)})[0]
            r = self.dec.run(None, {"inputs_embeds": x, "encoder_hidden_states": hid, "encoder_attention_mask": mask,
                                    "use_cache_branch": np.array([step > 0]), **past})
            for n, v in zip(self.outs[1:], r[1:]):
                k = n.replace("present", "past_key_values")
                if step == 0 or ".decoder." in k:          # the encoder part is computed once, on the first step
                    past[k] = v
            if step == 0:
                nxt = 0                                     # forced <s>
            else:
                logits = r[0][0, -1].astype(np.float32)
                pre = tuple(out[-(NGRAM - 1):])
                for j in range(len(out) - NGRAM + 1):
                    if tuple(out[j:j + NGRAM - 1]) == pre:
                        logits[out[j + NGRAM - 1]] = -np.inf
                nxt = int(logits.argmax())
            out.append(nxt)
            if nxt == 2 and step > 0:
                break
        return self.tok.decode(out)


def describe(jpeg):
    """Describe one JPEG. Returns {"text", "model", "secs"} like describe.ask. Raises OSError if not downloaded."""
    global _model
    if not ready():
        raise OSError("The built-in model isn't downloaded")
    img = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise ValueError("unreadable picture")
    with _lock:
        t = time.time()
        if _model is None:
            _model = _Florence(DIR)
        f = _model.features(img)
        cap = _model.answer(f, "<MORE_DETAILED_CAPTION>")
        txt = _model.answer(f, "<OCR>")
    txt = " ".join(txt.split())
    if len(txt.replace(" ", "")) < 3:                       # a stray letter or digit: there's no text
        txt = "none"
    return {"text": f"{cap}\nText: {txt}", "model": NAME, "secs": round(time.time() - t, 1)}
