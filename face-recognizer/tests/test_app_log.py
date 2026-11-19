"""The app log: every line starts with the time in UTC, however the text arrives in pieces."""
import io
import re

import app


def test_each_line_stamped_once():
    f = io.StringIO()
    log = app.Stamped(f)
    log.write("one")
    log.write(" more\ntwo\n")
    log.write("\n")
    log.write("three\nfour")
    log.flush()
    lines = f.getvalue().split("\n")
    stamp = r"\d\d:\d\d:\d\d "
    assert re.fullmatch(stamp + "one more", lines[0])
    assert re.fullmatch(stamp + "two", lines[1])
    assert lines[2] == ""
    assert re.fullmatch(stamp + "three", lines[3])
    assert re.fullmatch(stamp + "four", lines[4])


def held_by_this_test():
    return sum(range(60_000_000))            # one call into C: the interpreter's lock is held until it returns


def test_what_a_stuck_process_was_doing_goes_to_the_log(tmp_path):
    """The app's watchdog is a thread of the process: when a library call never returns with the interpreter's lock
    held (2026-11-19: a graphics driver installed under the app), it stops too. faulthandler's timer needs no
    interpreter: it writes every thread's stack to the log once the interpreter hasn't run for a while."""
    path = tmp_path / "app.log"
    with open(path, "a", encoding="utf-8") as f:
        stop = app.watch_interpreter(f, after=0.25)
        try:
            held_by_this_test()              # longer than 0.25 s on any computer; nothing else can run meanwhile
        finally:
            stop()
    text = path.read_text("utf-8")
    assert "Timeout (0:00:00.25" in text and "held_by_this_test" in text       # where the process was
    assert "Thread 0x" in text                                                 # and every thread
    assert app.watch_interpreter(io.StringIO()) is None      # not a real file: nothing to write to without the interpreter
    assert app.watch_interpreter(None) is None               # no console and no log (pythonw)
