"""The tracker in a process of its own (trackerlink.py, app.start_tracker): what the two processes ask of each other
is answered only to the app's own processes; the app takes only the process it started for the tracker, starts it
again when it ends, and ends it; and a file both processes rewrite is locked across processes."""
import json
import socket
import subprocess
import sys
import textwrap
import threading
import time

import pytest

import app
import camstream
import paths
import tracker
import trackerlink
from test_servers import FakeEngine, label_url, request  # noqa: F401  (label_url: a fixture)


@pytest.fixture
def link(monkeypatch):
    """This run's secret, as the app makes it; everything back as it was afterwards."""
    monkeypatch.setattr(trackerlink, "TOKEN", "t" * 64)
    monkeypatch.setattr(trackerlink, "REMOTE", False)
    monkeypatch.setattr(trackerlink, "TRACKER", None)
    monkeypatch.setattr(trackerlink, "LABEL", None)
    camstream.reset()
    yield {"X-Face-Tool": "1", "X-Face-Link": "t" * 64}
    camstream.reset()


@pytest.fixture
def track_url(tmp_path, serve, link):
    return serve(tracker.make_handler(FakeEngine(tmp_path)))


def test_only_the_apps_own_processes_get_the_trackers_local_answers(track_url, link, monkeypatch):
    monkeypatch.setattr(camstream, "start_heartbeat", lambda *a, **k: None)
    request(track_url, "/api/inview", json.dumps({"names": ["Ann"], "readout": []}).encode())
    for path in ("/api/local/inview", "/api/local/devices"):
        assert request(track_url, path)[0] == 403                                   # a program without the secret
        assert request(track_url, path, headers={"X-Face-Tool": "1"})[0] == 403
        assert request(track_url, path, headers={"X-Face-Tool": "1", "X-Face-Link": "wrong"})[0] == 403
        assert request(track_url, path, headers=link)[0] == 200
    names = json.loads(request(track_url, "/api/local/inview", headers=link)[2])["names"]
    assert list(names) == ["Ann"]
    assert set(json.loads(request(track_url, "/api/local/devices", headers=link)[2])) == {"devices", "decisions"}
    assert request(track_url, "/api/local/nothing", headers=link)[0] == 404
    # the peer service: only at the app's own word; without a shared database there is no key and it stays off
    body = json.dumps({"run": True, "label": "http://127.0.0.1:8765/"}).encode()
    assert request(track_url, "/api/local/peer", body)[0] == 403
    monkeypatch.setattr(tracker, "_cluster_key", lambda: None)
    code, _, got = request(track_url, "/api/local/peer", body, headers=link)
    assert code == 200 and json.loads(got) == {"running": False}
    assert trackerlink.LABEL == "http://127.0.0.1:8765/"       # where it asks for a photo and says "saved"
    request(track_url, "/api/local/peer", json.dumps({"run": False, "label": "http://evil.example/"}).encode(), headers=link)
    assert trackerlink.LABEL == "http://127.0.0.1:8765/"       # only an address on this computer is taken


def test_hello_proves_the_process_without_being_handed_the_secret(track_url, link, monkeypatch):
    seen = []
    real = tracker.local_get
    monkeypatch.setattr(tracker, "local_get", lambda path, query, headers, engine:
                        (seen.append(headers.get("X-Face-Link")), real(path, query, headers, engine))[1])
    assert trackerlink.hello(track_url) is True
    assert seen == [None]                                       # the asker did not send the secret to find out
    monkeypatch.setattr(tracker, "local_get", lambda path, query, headers, engine: ({"proof": "0" * 64}, 200))
    assert trackerlink.hello(track_url) is False                # a program on that port that isn't ours
    monkeypatch.setattr(tracker, "local_get", real)
    assert trackerlink.hello("http://127.0.0.1:9/") is False    # nothing there
    monkeypatch.setattr(trackerlink, "TOKEN", "")
    assert trackerlink.hello(track_url) is False                # no secret made: nothing is ours


def test_the_label_side_asks_the_tracker_over_the_link_and_shrugs_when_it_is_down(track_url, link, monkeypatch):
    monkeypatch.setattr(camstream, "start_heartbeat", lambda *a, **k: None)
    camstream.set_in_view(["Ben"])
    assert list(trackerlink.in_view()) == ["Ben"]               # the tracker in this process: its memory, as before
    monkeypatch.setattr(trackerlink, "REMOTE", True)
    assert trackerlink.in_view() == {} and trackerlink.devices() == ({}, {})       # its process isn't up yet
    assert trackerlink.peer_service(True, None, None, None) == (False, None)
    monkeypatch.setattr(trackerlink, "TRACKER", track_url + "/")
    assert list(trackerlink.in_view()) == ["Ben"]
    assert isinstance(trackerlink.devices()[0], dict)
    monkeypatch.setattr(tracker, "_cluster_key", lambda: None)
    assert trackerlink.peer_service(True, None, None, None) == (False, None)
    monkeypatch.setattr(tracker, "local_peer", lambda body, headers: ({"running": False, "error": "port taken"}, 200))
    with pytest.raises(OSError, match="port taken"):            # its process couldn't take the port: the sync's to know
        trackerlink.peer_service(True, None, None, None)
    monkeypatch.setattr(trackerlink, "TRACKER", "http://127.0.0.1:9/")             # it is starting again
    assert trackerlink.in_view() == {} and trackerlink.devices() == ({}, {})
    # ... and that is never the error above: the sync stops sharing for good on an OSError (it did, the first time
    # the tracker's process was ended in a test: the camera never reached the other devices again)
    assert trackerlink.peer_service(True, None, None, None) == (False, None)
    with pytest.raises(OSError):                                # the caller decides: a step without sharing
        trackerlink._ask("http://127.0.0.1:9/", "/api/local/inview", timeout=1)


def test_the_labeling_tool_answers_the_trackers_process_only(label_url, link):  # noqa: F811
    photo = "/api/local/photo/" + "0" * 40
    assert request(label_url, photo)[0] == 403
    assert request(label_url, photo, headers={"X-Face-Tool": "1", "X-Face-Link": "wrong"})[0] == 403
    assert request(label_url, photo, headers=link)[0] == 404                       # ours, but no such photo here
    assert request(label_url, "/api/local/photo/../../secret", headers=link)[0] == 404   # by content hash only
    assert request(label_url, "/api/local/saved", b"{}")[0] == 403
    assert request(label_url, "/api/local/saved", b"{}", headers=link)[0] == 200


FAKE_CHILD = textwrap.dedent("""
    import hashlib, hmac, http.server, json, os, sys
    port, token = int(sys.argv[sys.argv.index("--tracker-child") + 1]), os.environ["FACE_LINK"]
    wrong = os.environ.get("FAKE_WRONG") == "1"

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            body = b'{"people": []}'
            if self.path.startswith("/api/local/hello"):
                key = b"someone else" if wrong else token.encode()
                body = json.dumps({"proof": hmac.new(key, self.path.split("n=")[1].encode(), hashlib.sha256).hexdigest()}).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
""")


@pytest.fixture
def child(tmp_path, monkeypatch):
    """app.start_tracker with a stand-in for the tracker's process (no models here): a small server that answers
    hello with the secret from its environment."""
    script = tmp_path / "fake_child.py"
    script.write_text(FAKE_CHILD, "utf-8")
    monkeypatch.setattr(app, "child_command", lambda port: [sys.executable, str(script), "--tracker-child", str(port)])
    monkeypatch.setattr(app, "OWN", {})
    monkeypatch.setattr(app, "TRACKER", {"child": None, "stopping": False, "starts": []})
    monkeypatch.setattr(app, "CHILD_LOOK", 0.2)
    monkeypatch.setattr(trackerlink, "TOKEN", "")
    monkeypatch.setattr(trackerlink, "REMOTE", False)
    monkeypatch.setattr(trackerlink, "TRACKER", None)
    monkeypatch.delenv("FACE_LINK", raising=False)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    yield port
    app.TRACKER["stopping"] = True
    app.stop_tracker()


def test_the_app_starts_the_tracker_as_its_own_process_and_starts_it_again_when_it_ends(child, monkeypatch):
    again = []
    monkeypatch.setattr(app, "run_programs", lambda keys=("track", "label"): again.append(tuple(keys)))
    monkeypatch.setitem(app.STATUS, "track", {})
    url = app.start_tracker("Face tracker", child)
    assert url == f"http://127.0.0.1:{child}/" and trackerlink.REMOTE and trackerlink.TRACKER == url
    assert len(trackerlink.TOKEN) == 64 and app.OWN[url].is_alive()             # in OWN: the camera is answered for it
    app.STATUS["track"] = {"url": url}
    app.TRACKER["child"].proc.kill()                                              # it crashed
    end = time.time() + 10
    while not again and time.time() < end:
        time.sleep(0.1)
    assert again == [("track",)] and app.STATUS["track"] == {}                  # started again, by itself
    assert url not in app.OWN and trackerlink.TRACKER is None


def test_a_process_that_cant_show_it_is_ours_is_not_taken_for_the_tracker(child, monkeypatch):
    monkeypatch.setenv("FAKE_WRONG", "1")
    monkeypatch.setattr(app, "CHILD_START", 3)
    with pytest.raises(OSError, match="didn't answer"):
        app.start_tracker("Face tracker", child)
    assert not app.OWN and trackerlink.TRACKER is None


def test_the_tracker_is_ended_with_the_app_and_not_started_without_end(child, monkeypatch):
    monkeypatch.setattr(app, "run_programs", lambda keys=(): None)
    url = app.start_tracker("Face tracker", child)
    proc = app.TRACKER["child"].proc
    app.TRACKER["stopping"] = True
    app.stop_tracker()
    assert proc.poll() is not None and url not in app.OWN and app.TRACKER["child"] is None
    monkeypatch.setitem(app.TRACKER, "starts", [])
    assert [app.tracker_start_due(now=1000 + i) for i in range(7)] == [True] * 5 + [False] * 2
    assert app.tracker_start_due(now=1000 + app.CHILD_TRIES[1] + 10) is True      # later it may be tried again


def test_the_trackers_process_is_this_program_again(monkeypatch):
    import os
    monkeypatch.setitem(app.SHELL, "url", "http://127.0.0.1:8764/")
    cmd = app.child_command(8766)
    assert cmd[0] == sys.executable and cmd[-5:] == ["--tracker-child", "8766", str(os.getpid()), "8764", str(app.CAMS_PORT)]
    assert cmd[1].endswith("app.py")                             # from source: the script too (the built app: itself)


def test_a_file_two_processes_rewrite_is_locked_across_them(tmp_path):
    """alerts.json and the settings files are rewritten by the tracker's process and by the sync's."""
    held = tmp_path / "held"
    code = ("import paths, sys, time, pathlib\n"
            "lock = paths.SharedLock('both')\n"
            "with lock:\n"
            "    pathlib.Path(sys.argv[1]).write_text('x')\n"
            "    time.sleep(1.2)\n")
    env = dict(__import__("os").environ, FACE_HOME=str(paths.HOME), PYTHONPATH=str(paths.HERE))
    other = subprocess.Popen([sys.executable, "-c", code, str(held)], env=env)
    try:
        end = time.time() + 20
        while not held.exists() and time.time() < end:
            time.sleep(0.02)
        assert held.exists(), "the other process never took the lock"
        t = time.time()
        with paths.SharedLock("both"):                           # waits for the other process
            waited = time.time() - t
        assert waited > 0.5, waited
    finally:
        other.wait(10)
    lock, order = paths.SharedLock("threads"), []               # and this process's threads, as before

    def hold():
        with lock:
            order.append("in")
            time.sleep(0.2)
            order.append("out")
    threads = [threading.Thread(target=hold) for _ in range(3)]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    assert order == ["in", "out"] * 3
    with paths.SharedLock("another"), paths.SharedLock("both"):  # different files: no waiting on each other
        pass


def test_the_cameras_page_gets_a_port_of_its_own_in_the_trackers_process(tmp_path, link, monkeypatch):
    """The window gives each address a process of its own; the Cameras page and the tracker's page were one address.
    The tracker's process serves the same pages on a second port, and the app learns of it over the link."""
    import localguard
    monkeypatch.setattr(tracker, "CAMERAS", {})
    handler = tracker.make_handler(FakeEngine(tmp_path))
    first = localguard.bind(0)
    first.RequestHandlerClass = handler
    threading.Thread(target=first.serve_forever, daemon=True).start()
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    second = tracker.also_serve(handler, port)
    try:
        assert second is not None and tracker.CAMERAS == {"port": port}
        own = f"http://127.0.0.1:{port}"
        code, headers, body = request(own, "/cameras")
        assert code == 200 and b"cameras_ui.js" in body                         # the page, from its own address
        assert request(own, "/api/cameras", headers={"X-Face-Tool": "1"})[0] == 200         # and its answers
        assert request(own, "/api/cameras", headers={"Host": f"127.0.0.1:{first.server_address[1]}"})[0] == 403
        monkeypatch.setattr(trackerlink, "REMOTE", True)
        monkeypatch.setattr(trackerlink, "TRACKER", f"http://127.0.0.1:{first.server_address[1]}/")
        assert trackerlink.cameras_port() == port
        assert tracker.also_serve(handler, port) is None                         # taken: the page stays where it was
    finally:
        second.shutdown()
        second.server_close()
        first.shutdown()
        first.server_close()
    monkeypatch.setattr(trackerlink, "TRACKER", None)
    assert trackerlink.cameras_port() is None
    # the tab page may frame only the ports its policy names: the Cameras page's own must be one of them, and it
    # must not be one the app's other listeners use (peer service +3, Spotify's answer +4)
    assert app.CAMS_PORT in app.TOOL_PORTS and app.CAMS_PORT - app.SHELL_PORT == 5
