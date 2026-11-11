"""fetch_models.py: the first-start download, against a local server (no internet): one file out of a zip by
range requests, checksums, falling back to the next source, and Git LFS placeholders."""
import hashlib
import http.server
import io
import zipfile

import pytest

import fetch_models as M

ZIP_ERRORS = (OSError, zipfile.BadZipFile)       # fetch() treats either as "try the next source"

PAYLOAD = bytes(range(256)) * 400                 # 100 KB "model"
SHA = hashlib.sha256(PAYLOAD).hexdigest()


def _zip(method):
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w", method) as z:
        z.writestr("other/readme.txt", "hello")
        z.writestr("w600k_r50.onnx", PAYLOAD)
    return b.getvalue()


FILES = {"/stored.zip": _zip(zipfile.ZIP_STORED), "/deflated.zip": _zip(zipfile.ZIP_DEFLATED),
         "/model.onnx": PAYLOAD, "/wrong.onnx": b"x" * len(PAYLOAD)}


class _Files(http.server.BaseHTTPRequestHandler):
    """Serves FILES, with Range support (206) unless the path ends in ?norange."""
    def log_message(self, *a):
        pass

    def _answer(self, body_wanted):
        path, _, q = self.path.partition("?")
        data = FILES.get(path)
        if data is None:
            self.send_error(404)
            return
        rng = self.headers.get("Range")
        if rng and q != "norange":
            a, b = rng.split("=")[1].split("-")
            part = data[int(a):int(b) + 1]
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {a}-{int(a) + len(part) - 1}/{len(data)}")
        else:
            part = data
            self.send_response(200)
        self.send_header("Content-Length", str(len(part)))
        self.end_headers()
        if body_wanted:
            self.wfile.write(part)

    def do_GET(self):
        self._answer(True)

    def do_HEAD(self):
        self._answer(False)


@pytest.mark.parametrize("zipname", ["stored.zip", "deflated.zip"])
def test_zip_member_by_range_requests(serve, zipname):
    url = serve(_Files)
    r, unpack = M._zip_member(f"{url}/{zipname}", "w600k_r50.onnx", {})
    with r:
        got = b"".join(unpack(c) for c in iter(lambda: r.read(4096), b""))
    assert got == PAYLOAD


def test_zip_member_needs_range_support(serve):
    url = serve(_Files)
    with pytest.raises(ZIP_ERRORS):
        M._zip_member(f"{url}/stored.zip?norange", "w600k_r50.onnx", {})


@pytest.fixture
def one_model(tmp_path, monkeypatch, serve):
    """fetch_models set up for one file, "test.onnx", with a local server; returns that server's URL."""
    url = serve(_Files)
    monkeypatch.setattr(M, "MODELS", tmp_path / "models")
    monkeypatch.setattr(M, "FILES", {"test.onnx": (len(PAYLOAD), SHA)})
    monkeypatch.setattr(M, "WAIT", 0)
    return url


def test_fetch_from_the_publishers_zip(one_model, monkeypatch):
    monkeypatch.setattr(M, "ORIGIN", {"test.onnx": (f"{one_model}/deflated.zip", "w600k_r50.onnx")})
    monkeypatch.setattr(M, "_source", lambda name, attempt: pytest.fail("the fallback wasn't needed"))
    assert M.fetch() == ["test.onnx"]
    assert (M.MODELS / "test.onnx").read_bytes() == PAYLOAD and M.needed() == []


def test_a_changed_file_falls_back_to_the_release(one_model, monkeypatch):
    monkeypatch.setattr(M, "ORIGIN", {"test.onnx": f"{one_model}/wrong.onnx"})          # checksum differs
    monkeypatch.setattr(M, "_source", lambda name, attempt: (f"{one_model}/model.onnx", {}))
    seen = []
    M.fetch(lambda done, total, name: seen.append(done))
    assert (M.MODELS / "test.onnx").read_bytes() == PAYLOAD
    assert not list(M.MODELS.glob("*.download")) and seen[-1] == len(PAYLOAD)


def test_no_good_source_raises_and_leaves_nothing(one_model, monkeypatch):
    monkeypatch.setattr(M, "ORIGIN", {})
    monkeypatch.setattr(M, "_source", lambda name, attempt: (f"{one_model}/wrong.onnx", {}))
    with pytest.raises(OSError):
        M.fetch()
    assert not (M.MODELS / "test.onnx").exists() and not list(M.MODELS.glob("*.download"))


def test_an_optional_file_that_cant_be_fetched_is_skipped(one_model, monkeypatch):
    monkeypatch.setattr(M, "ORIGIN", {})
    monkeypatch.setattr(M, "OPTIONAL", {"test.onnx"})
    monkeypatch.setattr(M, "_source", lambda name, attempt: (f"{one_model}/missing.onnx", {}))     # 404
    assert M.fetch() == [] and M.needed() == ["test.onnx"]           # no error; tried again next time
    assert not list(M.MODELS.glob("*.download"))


def test_placeholders_count_as_missing(one_model):
    M.MODELS.mkdir()
    (M.MODELS / "test.onnx").write_bytes(b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 1\n")
    assert M.is_placeholder(M.MODELS / "test.onnx") and M.needed() == ["test.onnx"]


def test_a_file_of_the_right_size_but_wrong_contents_is_fetched_again(one_model):
    M.MODELS.mkdir()
    (M.MODELS / "test.onnx").write_bytes(b"x" * len(PAYLOAD))       # same size, not the model
    assert M.needed() == ["test.onnx"]


def test_a_checked_file_isnt_hashed_again_until_it_changes(one_model, monkeypatch):
    M.MODELS.mkdir()
    f = M.MODELS / "test.onnx"
    f.write_bytes(PAYLOAD)
    hashed = []
    real = M.sha256_of
    monkeypatch.setattr(M, "sha256_of", lambda p: hashed.append(p) or real(p))
    assert M.needed() == [] and len(hashed) == 1                       # checked once ...
    assert M.needed() == [] and len(hashed) == 1                       # ... then remembered (size and time)
    f.write_bytes(b"y" * len(PAYLOAD))                                 # replaced: same size, new time
    import os
    os.utime(f, ns=(f.stat().st_atime_ns, f.stat().st_mtime_ns + 10_000_000))
    assert M.needed() == ["test.onnx"] and len(hashed) == 2


def test_download_checked_refuses_a_wrong_file_and_keeps_nothing(one_model, tmp_path):
    dest = tmp_path / "w" / "model.bin"
    with pytest.raises(OSError):
        M.download_checked(f"{one_model}/wrong.onnx", dest, len(PAYLOAD), SHA)
    assert not dest.exists() and not list(dest.parent.glob("*.download"))
    assert M.download_checked(f"{one_model}/model.onnx", dest, len(PAYLOAD), SHA) == dest
    assert dest.read_bytes() == PAYLOAD
    assert M.download_checked("http://127.0.0.1:9/never-asked", dest, len(PAYLOAD), SHA) == dest   # already there
    dest.write_bytes(b"z" * len(PAYLOAD))
    with pytest.raises(OSError):
        M.download_checked(f"{one_model}/model.onnx", dest, len(PAYLOAD), SHA)
