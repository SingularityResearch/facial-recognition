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
    monkeypatch.setattr(hardware, "_port_open", lambda port, host="127.0.0.1", timeout=0.25: port == 1234 and host == "127.0.0.1")
    monkeypatch.setattr(describe, "server_models", lambda url=None, key=None: ([("qwen-vl", True), ("llama", False)], ["qwen-vl"]))
    monkeypatch.setattr(describe, "config", lambda: {"url": "http://127.0.0.1:1234", "key": ""})
    monkeypatch.setattr(hardware, "_installed_ai_apps", lambda: ["LM Studio", "Ollama"])
    found = hardware.ai_apps(max_age=0)
    lm = next(e for e in found if e["app"] == "LM Studio")
    assert lm["running"] and lm["models"] == 2 and lm["vision"] == ["qwen-vl"] and "qwen-vl" in lm["note"]
    assert lm["lan"] == []                                # this test's port answers on 127.0.0.1 only
    assert any(e["app"] == "Ollama" and not e["running"] for e in found)
    assert sum(e["app"] == "LM Studio" for e in found) == 1              # running, so not listed again as installed


def test_an_nvidia_card_without_cuda_is_explained(monkeypatch):
    monkeypatch.setattr(hardware, "describe", lambda: {"gpus": ["NVIDIA GeForce RTX 4060"], "onnxruntime": ["CPU"]})
    assert "setup.sh" in hardware.gpu_note()
    monkeypatch.setattr(hardware, "describe", lambda: {"gpus": ["NVIDIA GeForce RTX 4060"], "onnxruntime": ["CUDA", "CPU"]})
    assert hardware.gpu_note() == ""


def test_lan_addresses_are_this_computers_own():
    for ip in hardware.lan_addresses():
        assert not ip.startswith("127.") and ip.count(".") == 3


def test_lmstudio_network_merges_one_computers_addresses(monkeypatch):
    monkeypatch.setattr(hardware, "mdns_services", lambda timeout=1.5: [])       # no question to the real network,
    monkeypatch.setattr(hardware, "lmstudio_port", lambda: None)                 # no look at a real LM Studio
    monkeypatch.setattr(hardware, "_adapters", lambda max_age=300: None)         # as on a system that doesn't say
    monkeypatch.setattr(hardware, "lan_addresses", lambda: ["192.168.1.13"])
    monkeypatch.setattr(hardware.socket, "gethostbyname_ex", lambda h: (h, [], ["192.168.1.13", "192.168.1.6"]))
    up = {"192.168.1.6", "192.168.1.13", "192.168.1.18"}
    monkeypatch.setattr(hardware, "_port_open", lambda port, host="127.0.0.1", timeout=0.25: host in up)
    monkeypatch.setattr(hardware, "_name_of", lambda ip: {"192.168.1.18": "linux-box"}.get(ip, "windows-pc"))
    pc = [{"id": "qwen/qwen3.6-35b-a3b", "vision": True, "loaded": False, "quant": "Q4_K_M", "kind": "vlm"}]
    other = pc + [{"id": "openai/gpt-oss-20b", "vision": False, "loaded": True, "quant": "MXFP4", "kind": "llm"}]
    monkeypatch.setattr(hardware, "_lms_models", lambda url, timeout=3: other if "192.168.1.18" in url else pc)
    found = hardware.lmstudio_network(max_age=0)
    assert [(e["host"], e["ips"], e["this_computer"]) for e in found] == \
        [("windows-pc", ["192.168.1.6", "192.168.1.13"], True), ("linux-box", ["192.168.1.18"], False)]
    assert found[1]["urls"] == ["http://192.168.1.18:1234"] and found[1]["models"][1]["loaded"]


def test_an_announced_server_is_found_on_its_own_port(monkeypatch):
    monkeypatch.setattr(hardware, "own_addresses", lambda: ["192.168.1.13"])
    monkeypatch.setattr(hardware, "lmstudio_port", lambda: None)
    monkeypatch.setattr(hardware, "mdns_services", lambda timeout=1.5: [
        {"host": "linux-box", "ip": "192.168.1.18", "port": 18234, "txt": ["app=LM Studio"]}])
    monkeypatch.setattr(hardware, "_port_open", lambda port, host="127.0.0.1", timeout=0.25: (host, port) == ("192.168.1.18", 18234))
    monkeypatch.setattr(hardware, "_name_of", lambda ip: None)
    monkeypatch.setattr(hardware, "_lms_models", lambda url, timeout=3: [{"id": "m", "vision": True, "loaded": False,
                                                                          "quant": "Q4", "kind": "vlm"}])
    found = hardware.lmstudio_network(max_age=0)
    assert [(e["host"], e["urls"], e["announced"]) for e in found] == [("linux-box", ["http://192.168.1.18:18234"], True)]


def test_a_model_list_that_isnt_one_is_not_lm_studio(serve):
    """Any computer on the network can answer on LM Studio's port: an answer of another shape is taken as "not LM
    Studio", never as an error that fails the whole search."""
    import http.server
    import json

    def answering(body):
        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                data = json.dumps(body).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        return serve(H)
    assert hardware._lms_models(answering({"data": 5})) is None
    assert hardware._lms_models(answering([1, 2])) is None
    got = hardware._lms_models(answering({"data": [{"id": "a/b", "type": "vlm", "quantization": 7}, {"id": 5}, "x",
                                                    {"id": "c" * 300}]}))
    assert got == [{"id": "a/b", "vision": True, "loaded": False, "quant": None, "kind": "vlm"}]


def test_only_real_adapters_count_unless_the_system_is_fully_distributed(tmp_path, monkeypatch):
    """A VPN tunnel or a virtual switch isn't this computer's local network: its addresses are searched for LM Studio
    servers (and told to the other devices) only with the "Fully distributed system" setting on."""
    import lmstudio
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path)
    monkeypatch.setattr(lmstudio, "CONF", tmp_path / "lmstudio.json")
    monkeypatch.setattr(hardware, "_adapters", lambda max_age=300: [("192.168.1.13", False), ("100.90.1.2", True),
                                                                    ("172.28.0.1", True), ("127.0.0.1", False)])
    assert hardware.distributed() is False and hardware.own_addresses() == ["192.168.1.13"]
    lmstudio.set_share(tmp_path / "share")
    lmstudio.set_distributed(True)
    assert hardware.distributed() is True and lmstudio.share() == tmp_path / "share"     # each keeps the other
    assert hardware.own_addresses() == ["100.90.1.2", "172.28.0.1", "192.168.1.13"]
    assert hardware.own_addresses(every=False) == ["192.168.1.13"]
    lmstudio.set_share(tmp_path / "other")
    assert hardware.distributed() is True
    lmstudio.set_distributed(False)
    assert hardware.own_addresses() == ["192.168.1.13"] and lmstudio.share() == tmp_path / "other"
