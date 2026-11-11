"""cudasupport.py: CUDA fetched for the downloaded Linux app: when it applies, the download (checked, unpacked only
into its folder) against a local server, and the import finder that prefers the downloaded packages."""
import hashlib
import http.server
import io
import json
import threading
import zipfile

import cudasupport as C


def _wheel():
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("onnxruntime/__init__.py", "MARK = 'downloaded'\n")
        z.writestr("nvidia/cudnn/lib/libcudnn.so.9", b"\0lib")
        z.writestr("onnxruntime_gpu-1.24.4.dist-info/METADATA", "x")      # not one of the packages: left out
        z.writestr("../outside.txt", "must not be written")
    return buf.getvalue()


def _serve(data):
    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = data if self.path.endswith(".whl") else b""
            self.send_response(200 if body else 404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{srv.server_port}/"


def test_only_the_frozen_linux_app_with_an_nvidia_driver_needs_it(monkeypatch):
    assert not C.needed()                                   # the tests run from the code, not the built app
    monkeypatch.setattr(C.sys, "frozen", True, raising=False)
    monkeypatch.setattr(C, "nvidia_present", lambda: True)
    monkeypatch.setattr(C.sys, "platform", "linux")
    monkeypatch.setattr(C.os, "uname", lambda: type("u", (), {"machine": "x86_64"})(), raising=False)
    assert C.needed()
    monkeypatch.setenv("FACE_CUDA", "off")
    assert not C.needed()


def test_download_checks_and_unpacks_only_the_packages(tmp_path, monkeypatch):
    data = _wheel()
    monkeypatch.setattr(C, "PY", _serve(data))
    monkeypatch.setattr(C, "WHEELS", [("test-1.0-py3-none-any.whl", len(data), hashlib.sha256(data).hexdigest(), "aa")])
    monkeypatch.setattr(C, "SET", "testset")
    monkeypatch.setattr(C, "DIR", tmp_path / "cuda")
    monkeypatch.setattr(C, "READY", tmp_path / "cuda" / "ready.json")
    C._download()
    assert C.STATE["state"] == "ready" and C.ready()
    assert (tmp_path / "cuda" / "onnxruntime" / "__init__.py").exists()
    assert (tmp_path / "cuda" / "nvidia" / "cudnn" / "lib" / "libcudnn.so.9").exists()
    assert not (tmp_path / "cuda" / "onnxruntime_gpu-1.24.4.dist-info").exists()
    assert not (tmp_path / "outside.txt").exists() and not (tmp_path / "cuda.download").exists()


def test_a_wrong_file_is_refused(tmp_path, monkeypatch):
    data = _wheel()
    monkeypatch.setattr(C, "PY", _serve(data))
    monkeypatch.setattr(C, "WHEELS", [("test-1.0-py3-none-any.whl", len(data), "0" * 64, "aa")])
    monkeypatch.setattr(C, "DIR", tmp_path / "cuda")
    monkeypatch.setattr(C, "READY", tmp_path / "cuda" / "ready.json")
    C._download()
    assert C.STATE["state"] == "failed" and "SHA-256" in C.STATE["error"]
    assert not (tmp_path / "cuda").exists() and not C.ready()


def test_the_finder_prefers_the_downloaded_packages_only(tmp_path, monkeypatch):
    (tmp_path / "onnxruntime").mkdir()
    (tmp_path / "onnxruntime" / "__init__.py").write_text("MARK = 'downloaded'\n")
    monkeypatch.setattr(C, "DIR", tmp_path)
    f = C._FromDir()
    spec = f.find_spec("onnxruntime", None)
    assert spec and str(tmp_path) in spec.origin
    assert f.find_spec("numpy", None) is None and f.find_spec("onnx", None) is None


def test_status_words(monkeypatch):
    monkeypatch.setitem(C.STATE, "state", "downloading")
    monkeypatch.setitem(C.STATE, "done", 5e8)
    monkeypatch.setitem(C.STATE, "total", 1.86e9)
    assert "0.50 of 1.86 GB" in C.status()
    monkeypatch.setitem(C.STATE, "state", "idle")
    assert C.status() == ""
    assert json.dumps(C.WHEELS)                              # pinned list is plain data


def test_the_card_and_driver_are_checked_first():
    ok, why = C.check({"name": "NVIDIA GeForce RTX 4060 Laptop GPU", "cc": 8.9, "driver": "580.178.04"})
    assert ok and "RTX 4060" in why and "580.178.04" in why
    ok, why = C.check({"name": "NVIDIA GeForce GTX 1080", "cc": 6.1, "driver": "470.256.02"})
    assert not ok and "525.60" in why
    ok, why = C.check({"name": "NVIDIA GeForce GTX 750 Ti", "cc": 5.0, "driver": "550.1"})
    assert not ok and "compute capability 5.0" in why
    ok, why = C.check({"name": None, "cc": None, "driver": None})
    assert not ok and "driver" in why


def test_nothing_is_downloaded_unless_it_applies(monkeypatch):
    import pytest
    with pytest.raises(ValueError):
        C.start_download()                                    # not the Linux app: refused, nothing started
    assert C.offer() == {"offer": False}
