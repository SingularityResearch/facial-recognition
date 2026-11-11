"""hardware.py: what this computer has and how the programs use it (the Settings tab's "This computer")."""
import hardware


def test_describe_lists_the_hardware():
    h = hardware.describe()
    assert {"system", "cpu", "threads", "memory_gb", "gpus", "npu", "onnxruntime"} <= set(h)
    assert h["threads"] >= 1 and isinstance(h["gpus"], list) and h["cpu"]


def test_import_workers_leave_room_and_can_be_set(monkeypatch):
    monkeypatch.delenv("FACE_IMPORT_WORKERS", raising=False)
    monkeypatch.setattr(hardware, "cpu_threads", lambda: 16)
    assert hardware.import_workers() == 12                          # at most 12
    monkeypatch.setattr(hardware, "cpu_threads", lambda: 4)
    assert hardware.import_workers() == 2                           # two threads left for the rest
    monkeypatch.setattr(hardware, "cpu_threads", lambda: 1)
    assert hardware.import_workers() == 2
    monkeypatch.setenv("FACE_IMPORT_WORKERS", "5")
    assert hardware.import_workers() == 5


def test_uses_says_where_each_job_runs():
    u = hardware.uses()
    assert {"device_setting", "devices", "decisions", "import_workers", "photos", "videos"} <= set(u)
