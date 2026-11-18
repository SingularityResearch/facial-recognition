"""The names of forgotten people: HOME/forgotten.json, a list of names.

When the owner forgets a person (persondata.forget) and there is a shared database, their name is kept here: it is
the instruction for the devices that haven't forgotten yet, which do so when the sync brings it (dbsync_home.py:
the forgotten table). While a name is here, what arrives under it is refused, so a page still running can't write a
forgotten person back (devicesettings.merge_signals, exprlog.add, musiclog.add). Giving the name to someone again
(LabelDB.act, rename) takes it out, here and through the sync on every device (unforget), so a device joining
later doesn't forget that new person.

This module uses nothing of the programs but paths, so any module can import it at its top. (These functions were
in dbsync_home.py, the sync's module, which the labels, the settings and the logs imported inside their functions
just to ask about a name.)"""
import json
import threading

import paths

FILE = "forgotten.json"          # in HOME: names forgotten here or applied from other devices
_lock = threading.Lock()


def names(strict=False):
    """Names forgotten here or on another device, and not given to anyone since (unforget). strict (the sync): a
    file that is there but can't be read raises OSError instead of counting as "no names", which the sync would
    send on as every forget withdrawn."""
    try:
        return {n for n in json.loads((paths.HOME / FILE).read_text("utf-8")) if isinstance(n, str)}
    except FileNotFoundError:
        return set()
    except (OSError, ValueError, TypeError) as e:
        if strict:
            raise OSError(f"can't read {FILE}: {e}") from e
        return set()


def note(name):
    """persondata.forget calls this: the forget goes to the other devices with the next sync."""
    with _lock:
        kept = names() | {name}
        paths.atomic_write(paths.HOME / FILE, json.dumps(sorted(kept)))


def unforget(name):
    """The name was given to someone again (LabelDB.act, rename), or another device said so: the earlier forget no
    longer applies. Without this the name stayed an instruction for good, and a device joining later (or back
    after a long time away) forgot whoever carried the name by then, on every device. True if it was forgotten."""
    with _lock:
        kept = names()
        if name not in kept:
            return False
        paths.atomic_write(paths.HOME / FILE, json.dumps(sorted(kept - {name})))
        return True
