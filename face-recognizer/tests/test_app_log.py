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
