"""lmstudio.py: LM Studio on this computer served on all interfaces, announced, and given the network's models."""
import json

import lmstudio as L


def test_settings_are_switched_to_all_interfaces(tmp_path, monkeypatch):
    cfg = tmp_path / "http-server-config.json"
    cfg.write_text(json.dumps({"port": 1234, "networkInterface": "127.0.0.1", "cors": True}))
    monkeypatch.setattr(L, "config_path", lambda: cfg)
    monkeypatch.setattr(L, "running", lambda: False)
    assert L.serve_everywhere() == ["its settings now serve on all network interfaces"]
    assert json.loads(cfg.read_text()) == {"port": 1234, "networkInterface": "0.0.0.0", "cors": True}
    assert L.serve_everywhere() == []                                    # already so: nothing to do


def test_a_server_on_this_computer_only_is_started_again_on_all_interfaces(monkeypatch):
    calls = []
    monkeypatch.setattr(L, "config_path", lambda: None)
    monkeypatch.setattr(L, "running", lambda: True)
    monkeypatch.setattr(L, "on_network", lambda: False)
    monkeypatch.setattr(L.hardware, "own_addresses", lambda: ["10.0.0.18"])
    monkeypatch.setattr(L, "_lms", lambda *a, timeout=60: calls.append(a) or type("R", (), {"returncode": 0})())
    assert "started again" in L.serve_everywhere()[0]
    assert calls[-1] == ("server", "start", "--port", str(L.port()), "--bind", "0.0.0.0", "--cors")


def test_missing_models_are_the_networks_vision_models_this_one_lacks(monkeypatch):
    monkeypatch.setattr(L, "local_models", lambda: [{"key": "qwen/qwen3.6-35b-a3b", "variant": "qwen/qwen3.6-35b-a3b@q4_k_m",
                                                      "vision": True, "size": 1}])
    net = [{"host": "here", "this_computer": True, "models": [{"id": "x/only-here", "vision": True, "quant": "Q8_0"}]},
           {"host": "shiva-ubuntu", "this_computer": False, "models": [
               {"id": "qwen/qwen3.6-35b-a3b", "vision": True, "quant": "Q4_K_M"},
               {"id": "qwen/qwen3.8-27b", "vision": True, "quant": "Q4_K_M"},
               {"id": "openai/gpt-oss-20b", "vision": False, "quant": "MXFP4"}]}]
    monkeypatch.setattr(L, "share", lambda: None)
    assert L.missing_models(net) == [{"variant": "qwen/qwen3.8-27b@q4_k_m", "from": "shiva-ubuntu", "in_share": False}]


def test_not_announced_twice(monkeypatch):
    started = []
    monkeypatch.setattr(L, "running", lambda: True)
    monkeypatch.setattr(L, "port", lambda: 1234)
    monkeypatch.setattr(L, "announced_already", lambda: True)              # e.g. an Avahi service file
    monkeypatch.setattr(L, "_announcer", lambda: started.append(1))
    L._active.update(port=None, by=None)
    L.announce()
    assert not started and "by this computer itself" in L.STATE["announced"]


def _computer(root, monkeypatch, with_model):
    """A fake computer: LM Studio's models and hub folders under root; with_model puts one vision model there."""
    models, hub = root / "models", root / "hub"
    if with_model:
        repo = models / "lmstudio-community" / "Qwen3.8-27B-GGUF"
        repo.mkdir(parents=True)
        (repo / "Qwen3.8-27B-Q4_K_M.gguf").write_bytes(b"weights" * 1000)
        (repo / "mmproj-Qwen3.8-27B-BF16.gguf").write_bytes(b"projector")
        (repo / "Qwen3.8-27B-Q8_0.gguf").write_bytes(b"another quantization: not this one")
        (hub / "qwen" / "qwen3.8-27b").mkdir(parents=True)
        (hub / "qwen" / "qwen3.8-27b" / "manifest.json").write_text(json.dumps({"dependencies": [{"sources": [
            {"type": "huggingface", "user": "lmstudio-community", "repo": "Qwen3.8-27B-GGUF"}]}]}))
    monkeypatch.setattr(L, "models_dir", lambda: models)
    monkeypatch.setattr(L, "hub_dir", lambda: hub)
    monkeypatch.setattr(L, "local_models", lambda: [{"key": "qwen/qwen3.8-27b", "variant": "qwen/qwen3.8-27b@q4_k_m",
                                                      "vision": True, "size": 1}] if with_model else [])
    return models, hub


def test_models_go_through_the_shared_folder(tmp_path, monkeypatch):
    shared = tmp_path / "nas" / "lmstudio-models"
    monkeypatch.setattr(L, "CONF", tmp_path / "lmstudio.json")
    L.set_share(shared)                                       # made if missing
    _computer(tmp_path / "a", monkeypatch, True)
    L.publish()
    index = json.loads((shared / "index.json").read_text())
    entry = index["qwen/qwen3.8-27b@q4_k_m"]
    assert sorted(f["path"].split("/")[-1] for f in entry["files"]) == ["Qwen3.8-27B-Q4_K_M.gguf", "mmproj-Qwen3.8-27B-BF16.gguf"]
    assert (shared / "hub" / "qwen" / "qwen3.8-27b" / "manifest.json").exists()

    models, hub = _computer(tmp_path / "b", monkeypatch, False)
    assert L.missing_models([])[0] == {"variant": "qwen/qwen3.8-27b@q4_k_m", "from": "the shared folder", "in_share": True}
    assert L.from_share("qwen/qwen3.8-27b@q4_k_m")
    got = models / "lmstudio-community" / "Qwen3.8-27B-GGUF"
    assert (got / "Qwen3.8-27B-Q4_K_M.gguf").read_bytes() == b"weights" * 1000
    assert not (got / "Qwen3.8-27B-Q8_0.gguf").exists() and (hub / "qwen" / "qwen3.8-27b" / "manifest.json").exists()


def test_a_damaged_shared_copy_is_refused(tmp_path, monkeypatch):
    import pytest
    shared = tmp_path / "share"
    monkeypatch.setattr(L, "CONF", tmp_path / "lmstudio.json")
    L.set_share(shared)
    _computer(tmp_path / "a", monkeypatch, True)
    L.publish()
    victim = shared / "models" / "lmstudio-community" / "Qwen3.8-27B-GGUF" / "Qwen3.8-27B-Q4_K_M.gguf"
    victim.write_bytes(b"weightz" * 1000)                     # same size, different bytes
    models, _ = _computer(tmp_path / "b", monkeypatch, False)
    with pytest.raises(OSError, match="SHA-256"):
        L.from_share("qwen/qwen3.8-27b@q4_k_m")
    assert not list(models.rglob("*.gguf"))                   # nothing half-copied left behind


def test_any_shared_folder_failure_falls_back_to_the_internet(monkeypatch):
    got = []
    monkeypatch.setattr(L, "lms_path", lambda: "lms")
    monkeypatch.setattr(L, "from_share", lambda v, st=None: (_ for _ in ()).throw(OSError("NAS went away")))
    monkeypatch.setattr(L, "_lms_get", lambda v, st: (got.append(v) or True, ""))
    L.STATE["get"] = None
    L.get_models(["qwen/qwen3.8-27b@q4_k_m"])
    for _ in range(100):
        if not L.STATE["get"]["running"]:
            break
        __import__("time").sleep(0.02)
    g = L.STATE["get"]
    assert got == ["qwen/qwen3.8-27b@q4_k_m"] and g["error"] is None
    assert g["done"] == ["qwen/qwen3.8-27b@q4_k_m (downloaded)"] and "NAS went away" in g["notes"][0]
