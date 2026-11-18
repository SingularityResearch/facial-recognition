"""The peer service (peer.py) without a database: which of a device's addresses is tried first, what the service is
set up to stand, what a refused request says about the clocks, and the warning before the clocks are too far apart.
The parts that need the shared database are in test_sync.py."""
import hashlib
import json
import socket
import ssl
import time

import pytest

import peer


def test_a_devices_addresses_are_tried_in_the_order_most_likely_to_answer(monkeypatch):
    """A device lists every address it has (a VPN's too, with "Fully distributed system"); each one that doesn't
    answer costs CONNECT seconds. First the one that answered last, then those on a network this computer is on.
    Never this computer's own address at its own port (a VPN client's, the same on every computer: asked there, a
    device reached its own service and the log blamed the other's certificate)."""
    monkeypatch.setattr(peer, "addresses", lambda: ["192.168.1.13"])
    monkeypatch.setattr(peer, "own", lambda: {"192.168.1.13", "100.96.0.1"})
    monkeypatch.setattr(peer, "port", lambda: 8767)
    monkeypatch.setattr(peer, "_good", {})
    assert peer._order("fp", ["10.8.0.2", "100.96.0.1", "192.168.1.20"], 8767) == ["192.168.1.20", "10.8.0.2"]
    assert peer._order("fp", ["100.96.0.1"], 8767) == []
    assert peer._order("fp", ["192.168.1.13"], 9767) == ["192.168.1.13"]        # a second copy on this computer
    listed = ["10.8.0.2", "100.96.0.1", "192.168.1.20"]
    assert peer._order("fp", listed) == ["192.168.1.20", "10.8.0.2", "100.96.0.1"]
    peer._answered("fp", "100.96.0.1")
    assert peer._order("fp", listed) == ["100.96.0.1", "192.168.1.20", "10.8.0.2"]
    assert peer._order("another device", listed)[0] == "192.168.1.20"          # remembered per device
    peer._answered("fp", "10.8.0.2", False)                                    # another address failing changes nothing
    assert peer._order("fp", listed)[0] == "100.96.0.1"
    peer._answered("fp", "100.96.0.1", False)                                  # the remembered one stopped answering
    assert peer._order("fp", listed) == ["192.168.1.20", "10.8.0.2", "100.96.0.1"]
    assert peer._order("fp", None) == [] and peer.CONNECT < peer.TIMEOUT


def test_a_device_tells_the_others_only_addresses_they_can_reach(monkeypatch):
    """Its container bridges (Docker's 172.17.0.1 is the same on every computer) are never told, whatever
    "Fully distributed system" says; a VPN's address is, when that setting is on."""
    import hardware
    monkeypatch.setattr(hardware, "_adapters", lambda max_age=300: [("192.168.1.13", False), ("100.96.0.1", True),
                                                                    ("172.17.0.1", "container")])
    monkeypatch.setattr(hardware, "distributed", lambda: True)
    monkeypatch.setattr(peer, "_addr_cache", {"at": 0.0, "list": []})
    assert hardware.own_addresses() == ["100.96.0.1", "172.17.0.1", "192.168.1.13"]    # as before: all its own
    assert peer.addresses() == ["100.96.0.1", "192.168.1.13"]
    assert peer.own() == {"100.96.0.1", "172.17.0.1", "192.168.1.13"}                  # told or not: all are its own
    monkeypatch.setattr(hardware, "distributed", lambda: False)
    assert hardware.reachable_addresses() == ["192.168.1.13"] == hardware.own_addresses()
    monkeypatch.setattr(hardware, "_adapters", lambda max_age=300: None)               # a system that doesn't say
    monkeypatch.setattr(hardware, "lan_addresses", lambda: ["192.168.1.13"])
    assert "192.168.1.13" in hardware.reachable_addresses()


def test_the_service_stands_many_requests_at_once_and_answers_without_waiting():
    assert peer._Server.request_queue_size >= 128        # Python's 5: on Windows the sixth waited a second
    assert peer._Handler.disable_nagle_algorithm is True


def test_a_refusal_for_the_time_says_how_far_apart_the_clocks_are():
    import re
    behind = peer._why(json.dumps({"error": "stale or missing time", "now": int(time.time()) - 300}).encode())
    assert re.search(r"its clock is (299|300|301) s behind this device's", behind) and str(peer.CLOCK_SKEW) in behind
    assert "ahead of this device's" in peer._why(json.dumps({"error": "x", "now": time.time() + 500}).encode())
    assert peer._why(b'{"error": "not signed by a device of this cluster"}') == "not signed by a device of this cluster"
    assert peer._why(b"<html>no</html>") == "<html>no</html>" and peer._why(b"[1]") == "[1]"


def test_clocks_drifting_apart_are_said_in_the_log_before_requests_are_refused(capsys, monkeypatch):
    monkeypatch.setattr(peer, "_clock_said", {})
    import email.utils
    head = b"HTTP/1.0 200 OK\r\nServer: FaceRecognizerPeer\r\nDate: Sat, 14 Nov 2026 12:00:00 GMT\r\nContent-Length: 2"
    at = email.utils.parsedate_to_datetime("Sat, 14 Nov 2026 12:00:00 GMT").timestamp()
    assert peer._clock_note("192.168.1.20", head, now=at + 5) == -5 and capsys.readouterr().out == ""
    assert peer._clock_note("192.168.1.20", head, now=at + 81) == -81
    said = capsys.readouterr().out
    assert "81 s behind" in said and "192.168.1.20" in said and str(peer.CLOCK_SKEW) in said
    peer._clock_note("192.168.1.20", head, now=at + 82)
    assert capsys.readouterr().out == ""                  # once an hour for a device, not at every request
    peer._clock_note("192.168.1.21", head, now=at - 90)
    assert "90 s ahead" in capsys.readouterr().out        # another device: its own line
    peer._clock_note("192.168.1.20", head, now=at + 82 + 3601)
    assert "behind" in capsys.readouterr().out
    assert peer._clock_note("x", b"HTTP/1.0 200 OK\r\nServer: s") is None and peer._clock_note("x", b"Date: junk") is None


def test_a_computer_with_a_long_name_still_gets_its_certificate(tmp_path, monkeypatch):
    """A certificate's name may hold 64 characters; "Face Recognizer " and a long computer name were more, and the
    device got no certificate and no peer service."""
    pytest.importorskip("cryptography")
    import facedb
    from cryptography import x509
    monkeypatch.setattr(peer, "CERT", tmp_path / "peer-cert.pem")
    monkeypatch.setattr(peer, "KEY", tmp_path / "peer-key.pem")
    monkeypatch.setattr(facedb, "device", lambda: {"id": "0" * 36, "name": "a-computer-with-a-very-long-name-" * 4})
    cert, key, fp = peer.certificate()
    made = x509.load_pem_x509_certificate(cert.read_bytes())
    cn = made.subject.get_attributes_for_oid(x509.oid.NameOID.COMMON_NAME)[0].value
    assert len(cn) == 64 and cn.startswith("Face Recognizer a-computer") and len(fp) == 64 and key.exists()
    assert peer.certificate()[2] == fp                               # made once, the same afterwards


def test_a_running_service_refuses_an_old_request_with_its_own_time(tmp_path, monkeypatch):
    """The whole way on this computer: TLS with the pinned certificate, a signed request answered, one signed five
    minutes ago refused with the service's time, so the asker can say how far apart the clocks are."""
    pytest.importorskip("cryptography")
    monkeypatch.setattr(peer, "CERT", tmp_path / "peer-cert.pem")
    monkeypatch.setattr(peer, "KEY", tmp_path / "peer-key.pem")
    key = b"k" * 32
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    svc = peer.Service(lambda h, f: b"\xff\xd8photo" if h == "ab" * 20 else None, lambda: key,
                       bind="127.0.0.1", listen_port=port).start()
    try:
        fp = peer.certificate()[2]
        assert peer._get("127.0.0.1", port, fp, "/photo/" + "ab" * 20, key) == b"\xff\xd8photo"
        assert peer._get("127.0.0.1", port, fp, "/photo/" + "cd" * 20, key) is None          # not one of its photos
        with pytest.raises(PermissionError, match="not signed"):
            peer._get("127.0.0.1", port, fp, "/photo/" + "ab" * 20, b"x" * 32)
        with pytest.raises(ssl.SSLError):
            peer._get("127.0.0.1", port, "0" * 64, "/photo/" + "ab" * 20, key)                # not its certificate
        # a request signed five minutes ago, sent as the asking device sends it
        import facedb
        me, when, path = facedb.device()["id"], str(int(time.time()) - 300), "/photo/" + "ab" * 20
        ctx = peer._context(ssl.PROTOCOL_TLS_CLIENT, ssl.CERT_NONE)
        with ctx.wrap_socket(socket.create_connection(("127.0.0.1", port), timeout=5), server_hostname="127.0.0.1") as c:
            assert hashlib.sha256(c.getpeercert(binary_form=True)).hexdigest() == fp
            c.sendall((f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nX-Peer-Device: {me}\r\n"
                       f"X-Peer-Time: {when}\r\nX-Peer-Sig: {peer.sign(key, 'GET', path, when, me)}\r\n\r\n").encode())
            data = b""
            while chunk := c.recv(65536):
                data += chunk
        head, _, body = data.partition(b"\r\n\r\n")
        assert b" 403 " in head.split(b"\r\n")[0] and abs(json.loads(body)["now"] - time.time()) < 5
        assert "apart is refused" in peer._why(body) and "clocks automatically" in peer._why(body)
    finally:
        svc.stop()


def test_the_services_private_key_is_kept_encrypted(tmp_path, monkeypatch):
    """Through 1.24.2 peer-key.pem lay in the private folder as it was. Now it is encrypted, its password kept as
    the app's other keys are (paths.protect); a key from before is encrypted where it is, with the same certificate;
    one that can't be opened any more is replaced, and the service starts."""
    pytest.importorskip("cryptography")
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ec
    import paths
    monkeypatch.setattr(peer, "CERT", tmp_path / "peer-cert.pem")
    monkeypatch.setattr(peer, "KEY", tmp_path / "peer-key.pem")
    cert, key, fp = peer.certificate()
    assert b"ENCRYPTED PRIVATE KEY" in key.read_bytes() and peer.key_password()
    kept = json.loads((tmp_path / "peer-key.json").read_text("utf-8"))["password"]
    assert paths.is_protected(kept) and peer.key_password() not in key.read_text("ascii")
    with pytest.raises((TypeError, ValueError)):
        serialization.load_pem_private_key(key.read_bytes(), None)             # no use without the password
    private = serialization.load_pem_private_key(key.read_bytes(), peer.key_password().encode())

    # a device coming from 1.24.2: the same key, as it lay there
    key.write_bytes(private.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                          serialization.NoEncryption()))
    (tmp_path / "peer-key.json").unlink()
    assert peer.certificate()[2] == fp and b"ENCRYPTED PRIVATE KEY" in key.read_bytes()
    again = serialization.load_pem_private_key(key.read_bytes(), peer.key_password().encode())
    assert again.private_numbers() == private.private_numbers()                # the same key: the others still know it

    def serves():
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        svc = peer.Service(lambda h, f: b"\xff\xd8photo", lambda: b"k" * 32, bind="127.0.0.1", listen_port=port).start()
        try:
            return peer._get("127.0.0.1", port, peer.certificate()[2], "/photo/" + "ab" * 20, b"k" * 32)
        finally:
            svc.stop()
    assert serves() == b"\xff\xd8photo"

    # the password is gone (the private folder came from another computer): a new certificate, said in the database
    (tmp_path / "peer-key.json").unlink()
    fp2 = peer.certificate()[2]
    assert fp2 != fp and peer.key_password() and serves() == b"\xff\xd8photo"
    # the key and its password don't fit (one was written, the other not): found when the service starts
    other = ec.generate_private_key(ec.SECP256R1())
    key.write_bytes(other.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                        serialization.BestAvailableEncryption(b"another password")))
    assert serves() == b"\xff\xd8photo" and peer.certificate()[2] not in (fp, fp2)
