"""The start-up check that decides whether a GPU (CUDA, CoreML) runs a model: same results as the CPU, and faster."""
import time
from types import SimpleNamespace

import numpy as np

import face_id as F

X = np.random.default_rng(1).uniform(-1, 1, (1, 3, 8, 8)).astype(np.float32)


def model(x):
    return x.reshape(1, -1)[:, :16] * 2.0


def slow(run, ms):
    def wrapped(x):
        time.sleep(ms / 1000)
        return run(x)
    return wrapped


def test_same_and_faster_uses_the_gpu():
    ok, why = F._gpu_check(model, slow(model, 5), X)
    assert ok, why


def test_different_results_stay_on_the_cpu():
    ok, why = F._gpu_check(lambda x: model(x) * 1.1, slow(model, 5), X)
    assert not ok and "different" in why


def test_not_faster_stays_on_the_cpu():
    ok, why = F._gpu_check(slow(model, 5), model, X)
    assert not ok and "not faster" in why


def test_broken_gpu_stays_on_the_cpu():
    def broken(x):
        raise RuntimeError("CUDA failure 35: driver too old")
    ok, why = F._gpu_check(broken, model, X)
    assert not ok and "failed" in why
    ok, _ = F._gpu_check(lambda x: model(x) * np.nan, slow(model, 5), X)
    assert not ok


def test_small_outputs_compared_by_size_not_direction():
    """A one-number output (the quality score) must match in value, not only in sign."""
    ok, _ = F._gpu_check(lambda x: np.array([[0.5]]), slow(lambda x: np.array([[0.7]]), 5), X)
    assert not ok


def test_test_input_fills_unknown_sizes():
    sess = SimpleNamespace(get_inputs=lambda: [SimpleNamespace(type="tensor(float)", shape=["batch", 3, 112, 112])])
    x = F._test_input(sess)
    assert x.shape == (1, 3, 112, 112) and x.dtype == np.float32
    assert np.array_equal(x, F._test_input(sess))                     # the same input every start
    sess = SimpleNamespace(get_inputs=lambda: [SimpleNamespace(type="tensor(int64)", shape=[1])])
    assert F._test_input(sess) is None


# ---- a GPU that is gone while the program runs (a graphics driver installed or reset under the app) ----------------
class Session:
    """Stands for a GPU session: what must never happen to one that failed is its teardown, which on a device that
    is gone never returns, with the interpreter's lock held (two apps hung that way on 2026-11-19)."""
    ended = []

    def __del__(self):
        Session.ended.append(self)


def lost_gpu(state):
    sess = Session()                                  # lives as long as its runner does

    def gpu(x):
        if state["lost"]:
            raise RuntimeError("DmlExecutionProvider CommandQueue.cpp(29): the device is gone")
        return "gpu" if sess else None
    return gpu


def test_a_model_whose_gpu_fails_goes_on_on_the_cpu_and_its_session_is_never_destroyed(monkeypatch, capsys):
    import gc
    made, said, state = [], [], {"lost": False}
    monkeypatch.setattr(F, "_cv_runner", lambda path: (made.append(path), lambda x: "cpu")[1])
    monkeypatch.setattr(F, "_LOST", [])
    Session.ended.clear()
    run = F._or_cpu(lost_gpu(state), "model.onnx", "Recognition", lambda: said.append("recognition"))
    assert run(1) == "gpu" and not made
    state["lost"] = True
    assert run(1) == "cpu"                            # the run that failed is answered by the CPU: nothing is lost
    assert run(2) == "cpu" and made == ["model.onnx"] and said == ["recognition"]      # taken over once
    gc.collect()
    assert not Session.ended and len(F._LOST) == 1    # kept for as long as the process lives
    assert "Recognition on the GPU failed" in capsys.readouterr().err
    state["lost"] = False
    assert run(3) == "cpu"                            # and no way back in this process: the device was another


def test_several_requests_failing_at_once_hand_over_once(monkeypatch):
    import threading
    made, said, state = [], [], {"lost": True}
    monkeypatch.setattr(F, "_cv_runner", lambda path: (made.append(path), time.sleep(0.05), lambda x: "cpu")[2])
    monkeypatch.setattr(F, "_LOST", [])
    run, out = F._or_cpu(lost_gpu(state), "model.onnx", "Quality", lambda: said.append(1)), []
    threads = [threading.Thread(target=lambda: out.append(run(0))) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert out == ["cpu"] * 8 and len(made) == 1 and said == [1] and len(F._LOST) == 1


def test_face_finding_keeps_its_failed_gpu_sessions_too(monkeypatch, capsys):
    """Faces._detect_at already went over to OpenCV when the GPU failed, and then let its sessions go: the last
    thing two processes did. Now they are kept."""
    import gc
    monkeypatch.setattr(F, "_LOST", [])
    Session.ended.clear()

    def rows(run, img, score):                        # as _yunet_rows: runs the detector's sessions
        return run(img)
    monkeypatch.setattr(F, "_yunet_rows", rows)
    faces = object.__new__(F.Faces)                   # without the models: only what _detect_at uses
    faces._yunet, faces.devices = (lost_gpu({"lost": True}), "GPU (DirectML, checked)"), {}
    faces._detector = lambda score: SimpleNamespace(setInputSize=lambda size: None, detect=lambda img: (0, None))
    assert faces._detect_at(np.zeros((90, 160, 3), np.uint8), 640) == []
    gc.collect()
    assert faces._yunet is None and faces.devices["detection"] == "CPU (OpenCV, after a GPU error)"
    assert len(F._LOST) == 1 and not Session.ended
    assert "Face finding on the GPU failed" in capsys.readouterr().err


def test_a_model_that_went_over_to_the_cpu_says_so_where_the_devices_are_shown(monkeypatch):
    monkeypatch.setattr(F, "LAST_DEVICES", {"recognition": "GPU (DirectML)", "quality": "GPU (DirectML)"})
    faces = object.__new__(F.Faces)
    faces.device, faces.devices = "GPU (DirectML)", {"recognition": "GPU (DirectML)", "quality": "GPU (DirectML)"}
    faces._on_cpu("quality")
    assert faces.devices == {"recognition": "GPU (DirectML)", "quality": "CPU (OpenCV, after a GPU error)"}
    assert faces.device == "GPU (DirectML)" and F.LAST_DEVICES["quality"] == "CPU (OpenCV, after a GPU error)"
    faces._on_cpu("recognition")
    assert faces.device == "CPU (OpenCV, after a GPU error)" == F.LAST_DEVICES["recognition"]
