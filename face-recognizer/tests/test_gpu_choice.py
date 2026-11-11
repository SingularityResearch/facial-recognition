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
