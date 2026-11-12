"""The tracker reloads people.npz when it changes, and tries again when a read fails (on Windows the file is briefly
locked while the labeling tool, or the shared database's sync, replaces it)."""
import threading
from types import SimpleNamespace

import face_id as F
import tracker


def test_a_failed_read_of_people_npz_is_tried_again(tmp_path, monkeypatch):
    people = tmp_path / "people.npz"
    people.write_bytes(b"x")
    monkeypatch.setattr(tracker, "PEOPLE", people)
    calls = []

    def load():
        calls.append(1)
        if len(calls) == 1:
            raise PermissionError(13, "Permission denied")
        return SimpleNamespace(names=["Ann", "Bob"])
    monkeypatch.setattr(F.Identifier, "load", staticmethod(load))
    engine = tracker.Engine.__new__(tracker.Engine)
    engine.lock, engine.ident, engine.people_mtime = threading.Lock(), None, None
    assert engine.people() == []                          # locked: nothing yet, and no crash
    assert engine.people() == ["Ann", "Bob"]              # the next request reads it
    assert engine.people() == ["Ann", "Bob"] and len(calls) == 2      # then only when it changes
