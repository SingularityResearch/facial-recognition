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


def test_ai_apps_reports_servers_and_installed_apps(monkeypatch):
    import describe
    monkeypatch.setattr(hardware, "_port_open", lambda port, host="127.0.0.1", timeout=0.25: port == 1234)
    monkeypatch.setattr(describe, "server_models", lambda url=None, key=None: ([("qwen-vl", True), ("llama", False)], ["qwen-vl"]))
    monkeypatch.setattr(describe, "config", lambda: {"url": "http://127.0.0.1:1234", "key": ""})
    monkeypatch.setattr(hardware, "_installed_ai_apps", lambda: ["LM Studio", "Ollama"])
    found = hardware.ai_apps(max_age=0)
    lm = next(e for e in found if e["app"] == "LM Studio")
    assert lm["running"] and lm["models"] == 2 and lm["vision"] == ["qwen-vl"] and "qwen-vl" in lm["note"]
    assert any(e["app"] == "Ollama" and not e["running"] for e in found)
    assert sum(e["app"] == "LM Studio" for e in found) == 1              # running, so not listed again as installed
