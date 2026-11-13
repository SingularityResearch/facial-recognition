"""The window watchdog: a page that stops answering gets its renderer stopped, then the app started again; a page
that answers, a page not loaded yet and a computer waking from sleep are left alone."""
import app


def run(times, alive, platform="linux", monkeypatch=None):
    monkeypatch.setattr(app.sys, "platform", platform)
    st, out = {}, []
    for t in times:
        out.append(app.watch_step(st, t, alive(t)))
    return out


def test_answering_page_left_alone(monkeypatch):
    assert run(range(0, 120, 5), lambda t: t - 2, monkeypatch=monkeypatch) == [None] * 24


def test_not_loaded_yet_left_alone(monkeypatch):
    assert run(range(0, 120, 5), lambda t: None, monkeypatch=monkeypatch) == [None] * 24


def test_frozen_page_renderer_stopped_then_app_restarted(monkeypatch):
    out = run(range(5, 100, 5), lambda t: 0.0, monkeypatch=monkeypatch)       # last word at 0
    assert out.index("kill") * 5 + 5 == app.HUNG_AFTER
    assert out.index("restart") * 5 + 5 == app.HUNG_AFTER + app.RESTART_AFTER
    assert out[:out.index("restart")].count("kill") == 1         # (the restart replaces the process)


def test_recovered_page_after_kill_not_restarted(monkeypatch):
    out = run(range(5, 120, 5), lambda t: 0.0 if t < 40 else t - 1, monkeypatch=monkeypatch)
    assert "kill" in out and "restart" not in out


def test_windows_restarts_without_kill(monkeypatch):
    out = run(range(5, 100, 5), lambda t: 0.0, platform="win32", monkeypatch=monkeypatch)
    assert "kill" not in out and "restart" in out


def test_sleep_is_not_a_freeze(monkeypatch):
    monkeypatch.setattr(app.sys, "platform", "linux")
    st = {}
    assert app.watch_step(st, 5, 4) is None
    assert app.watch_step(st, 3605, 4) == "reset"                 # an hour asleep: the watchdog's own clock jumped
    assert app.watch_step(st, 3610, 3605) is None


def test_camera_restart_at_most_hourly_and_only_in_the_window(monkeypatch, tmp_path):
    started = []
    monkeypatch.setattr(app, "HOME", tmp_path)
    monkeypatch.setattr(app, "restart_app", lambda: started.append(1))

    class Now:                                      # threading.Timer, at once
        def __init__(self, _s, fn):
            self.fn = fn

        def start(self):
            self.fn()
    monkeypatch.setattr(app.threading, "Timer", Now)
    monkeypatch.setattr(app, "WINDOW", None)
    assert app.restart_soon("test") is False and not started          # browser or test copy: never
    monkeypatch.setattr(app, "WINDOW", object())
    assert app.restart_soon("test") is True and started == [1]
    assert app.restart_soon("test") is False and started == [1]       # again within the hour: no


def test_restart_keeps_program_and_arguments(monkeypatch):
    seen = {}
    monkeypatch.setattr(app.sys, "platform", "linux")
    monkeypatch.setattr(app.os, "execve", lambda exe, args, env: seen.update(exe=exe, args=args, env=env))
    monkeypatch.setattr(app.sys, "argv", ["app.py", "--browser"])
    app.restart_app()
    assert seen["exe"] == app.sys.executable and seen["args"][-1] == "--browser"
    assert seen["env"]["PYINSTALLER_RESET_ENVIRONMENT"] == "1"


def test_windows_restart_new_copy_waits_for_the_old(monkeypatch):
    import subprocess
    seen = {}
    monkeypatch.setattr(app.sys, "platform", "win32")
    monkeypatch.setattr(app.sys, "argv", ["app.py", "--browser"])
    monkeypatch.setattr(subprocess, "Popen", lambda args, env, close_fds: seen.update(args=args, env=env))
    monkeypatch.setattr(app.os, "execve", lambda *a: seen.update(execve=True))

    class Exit(Exception):
        pass

    def _exit(code):
        raise Exit
    monkeypatch.setattr(app.os, "_exit", _exit)
    try:
        app.restart_app()
    except Exit:
        pass
    assert "execve" not in seen and seen["args"][-1] == "--browser"
    assert seen["env"]["FACE_RESTART_AFTER"] == str(app.os.getpid())


def test_wait_for_previous_takes_the_variable_away(monkeypatch):
    monkeypatch.setenv("FACE_RESTART_AFTER", "not a pid")
    app.wait_for_previous(timeout=0)
    assert "FACE_RESTART_AFTER" not in app.os.environ


def test_window_restarts_in_a_row_wait_longer():
    assert app.window_restart_due({}, 1000.0) == (True, 0)                  # the first: at once
    assert app.window_restart_due({"at": 1000, "n": 1}, 1060) == (False, 1)  # one a minute later: not yet
    assert app.window_restart_due({"at": 1000, "n": 1}, 1000 + app.WINDOW_BACKOFF) == (True, 1)
    assert app.window_restart_due({"at": 1000, "n": 3}, 1000 + app.WINDOW_BACKOFF * 3)[0] is False   # 4x now
    assert app.window_restart_due({"at": 1000, "n": 9}, 1000 + app.RESTART_EVERY - 1)[0] is False   # an hour, at most
    assert app.window_restart_due({"at": 1000, "n": 9}, 1000 + app.RESTART_EVERY) == (True, 0)      # starts over
    assert app.window_restart_due("junk", 5.0) == (True, 0)


def test_old_restart_file_counts_as_the_cameras(monkeypatch, tmp_path):
    monkeypatch.setattr(app, "HOME", tmp_path)
    (tmp_path / "restarted.json").write_text('{"at": 5, "why": "camera"}', "utf-8")
    assert app._restarts() == {"camera": {"at": 5, "why": "camera"}}
    app._note_restart("window", {"at": 7, "n": 1})
    assert app._restarts() == {"camera": {"at": 5, "why": "camera"}, "window": {"at": 7, "n": 1}}


def test_the_window_gets_room_for_more_open_files():
    """Linux and macOS: the app raises its limit on open files (its window's helper processes inherit it), never
    lowers one, and never fails over it. On Windows there is no such limit: None."""
    import sys

    import app
    if sys.platform == "win32":
        assert app.allow_more_open_files() is None
        return
    import resource
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    try:
        low = 256 if hard == resource.RLIM_INFINITY else min(256, hard)
        resource.setrlimit(resource.RLIMIT_NOFILE, (low, hard))
        want = 512 if hard == resource.RLIM_INFINITY else min(512, hard)
        assert app.allow_more_open_files(512) == want == resource.getrlimit(resource.RLIMIT_NOFILE)[0]
        assert app.allow_more_open_files(128) == want                       # already more: left as it is
    finally:
        resource.setrlimit(resource.RLIMIT_NOFILE, (soft, hard))


def test_the_windows_browser_flags_keep_what_is_set_and_are_added_once():
    """The flags that keep the window's pages running in the background and the camera's frames in ordinary memory
    are added to whatever is set already (a test's DevTools port), once."""
    import app
    env = {"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--remote-debugging-port=9335"}
    app.keep_running_in_background(env)
    for var in ("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "QTWEBENGINE_CHROMIUM_FLAGS"):
        assert "--disable-background-timer-throttling" in env[var] and "MediaFoundationD3D11VideoCapture" in env[var]
        assert env[var].count("--disable-features=") == 1         # one list: a second one would replace the first
    assert env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"].startswith("--remote-debugging-port=9335 ")
    before = dict(env)
    app.keep_running_in_background(env)
    assert env == before
