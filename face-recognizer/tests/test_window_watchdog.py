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


def test_restart_keeps_program_and_arguments(monkeypatch):
    seen = {}
    monkeypatch.setattr(app.os, "execve", lambda exe, args, env: seen.update(exe=exe, args=args, env=env))
    monkeypatch.setattr(app.sys, "argv", ["app.py", "--browser"])
    app.restart_app()
    assert seen["exe"] == app.sys.executable and seen["args"][-1] == "--browser"
    assert seen["env"]["PYINSTALLER_RESET_ENVIRONMENT"] == "1"
