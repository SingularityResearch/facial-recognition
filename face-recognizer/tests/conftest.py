"""Shared setup for the Python tests (run from face-recognizer\\: python -m pytest tests).

The tests never touch the real data\\ or %APPDATA%\\FaceLabels: before anything from the project is imported,
FACE_ROOT and FACE_HOME point at an empty temporary folder (no models, no data, as in CI or a new copy), and
each test that writes gets its own folders through the `data` and `home` fixtures. No test needs the models,
the network or a camera; servers they start listen on 127.0.0.1 on a free port.
"""
import http.server
import json
import os
import sys
import tempfile
import threading
from pathlib import Path

_TMP = Path(tempfile.mkdtemp(prefix="face-tests-"))
os.environ["FACE_ROOT"] = str(_TMP)
os.environ["FACE_HOME"] = str(_TMP / "home")
for var in ("FACE_MODEL", "FACE_LLM_URL", "GOOGLE_VISION_API_KEY", "FACE_DB", "FACE_DB_SCHEMA"):
    os.environ.pop(var, None)
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np  # noqa: E402
import pytest  # noqa: E402

import hardware  # noqa: E402

# No test asks the real system which network adapters it has (on Windows that starts PowerShell): "it doesn't say",
# unless a test gives its own list.
hardware._adapters = lambda max_age=300: None


@pytest.fixture
def data(tmp_path, monkeypatch):
    """An empty data\\ folder of its own (face_id.DATA)."""
    import face_id as F
    d = tmp_path / "data"
    d.mkdir()
    monkeypatch.setattr(F, "DATA", d)
    return d


@pytest.fixture
def home(tmp_path, monkeypatch):
    """An empty %APPDATA%\\FaceLabels of its own, for describe.py and websearch.py."""
    import describe
    import websearch
    h = tmp_path / "home"
    h.mkdir()
    for mod, names in ((describe, {"HOME": "", "CONFIG": "describe.json", "STORE": "descriptions.json"}),
                       (websearch, {"HOME": "", "KEY_FILE": "google_vision_key.txt", "AWS_FILE": "aws_rekognition.json"})):
        for attr, file in names.items():
            monkeypatch.setattr(mod, attr, h / file if file else h)
    monkeypatch.setattr(describe, "_saved", None)
    return h


def make_faces(data, n=6, thumbs=None, seed=0):
    """A tiny faces.npz: n random unit face prints (ArcFace size) from n // 2 photos, IDs f0..f{n-1}.
    thumbs: None (no pictures), or a list of bytes stored the old way, inside faces.npz."""
    rng = np.random.default_rng(seed)
    emb = rng.normal(size=(n, 512)).astype(np.float32)
    emb /= np.linalg.norm(emb, axis=1, keepdims=True)
    arrays = dict(emb=emb, model=np.asarray("arcface_w600k_r50"),
                  files=np.asarray([f"photo{i // 2}.jpg" for i in range(n)]),
                  fid=np.asarray([f"f{i}" for i in range(n)]), photo=np.asarray([f"h{i // 2}" for i in range(n)]),
                  box=np.tile(np.asarray([[10, 10, 80, 80, 0.99]], np.float32), (n, 1)),
                  quality=np.full(n, 0.9, np.float32))
    if thumbs is not None:
        arrays["thumbs"] = np.frombuffer(b"".join(thumbs), np.uint8)
        arrays["thumb_off"] = np.cumsum([0] + [len(t) for t in thumbs]).astype(np.int64)
    np.savez(data / "faces.npz", **arrays)
    (data / "photos.json").write_text(json.dumps({f"h{k}": {"path": ""} for k in range((n + 1) // 2)}), "utf-8")
    return emb


@pytest.fixture
def serve():
    """serve(handler_class) -> base URL of a server on 127.0.0.1 answering with it (stopped after the test)."""
    servers = []

    def start(handler):
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        servers.append(srv)
        return f"http://127.0.0.1:{srv.server_address[1]}"
    yield start
    for srv in servers:
        srv.shutdown()
        srv.server_close()
