"""The Raspberry Pi's own code flow (pi.py): entered on a Raspberry Pi only, and there a frame never waits for a
face's print. No models and no Pi needed.

The owner's rule (2026-11-20): nothing changes how the other hardware works to make the Pi easier."""
import threading

import cv2
import numpy as np

import face_id as F
import pi
import tracker
from test_servers import request, track  # noqa: F401  (track: a fixture)


class Faces:
    """Stands in for face_id.Faces: a detector that finds the boxes it is given, a quality and a print that say
    which crop they were asked for."""
    expression_model = None
    detect = F.Faces.detect                       # the general flow's own way through a frame

    def __init__(self):
        self.boxes, self.embeds = [], 0

    def _detect_at(self, bgr, side, score=None):
        rows = []
        for x, y, w, s in self.boxes:
            kps = [x + .3 * w, y + .4 * w, x + .7 * w, y + .4 * w, x + .5 * w, y + .6 * w, x + .35 * w, y + .8 * w, x + .65 * w, y + .8 * w]
            rows.append(np.array([x, y, w, w, *kps, s], np.float32))
        return rows

    def quality(self, crop):
        return float(crop[0, 0, 0]) / 255

    def embed(self, crop):
        self.embeds += 1
        v = np.zeros(8, np.float32)
        v[int(crop[0, 0, 0]) % 8] = 1
        return v

    def expression(self, crop):
        return None

    def face_mesh(self, img, row):
        return None, None

    def poses(self, img):
        return []

    def hands(self, img):
        return []


def frame(*shades):
    """A 640x360 picture whose left half has one shade and right half another: a face's crop says where it was."""
    img = np.zeros((360, 640, 3), np.uint8)
    img[:, :320], img[:, 320:] = shades[0], shades[-1]
    return img


def test_only_a_raspberry_pi_enters_the_pis_flow():
    pi4, pi5 = "Raspberry Pi 4 Model B Rev 1.4", "Raspberry Pi 5 Model B Rev 1.0"
    assert pi.here({}, "linux", "aarch64", pi4) and pi.here({}, "linux", "arm64", pi5)
    for system, machine, name in (("win32", "AMD64", ""), ("win32", "ARM64", ""), ("darwin", "arm64", ""),
                                  ("linux", "x86_64", ""), ("linux", "x86_64", pi4), ("linux", "aarch64", "Some other board"),
                                  ("linux", "aarch64", "")):
        assert not pi.here({}, system, machine, name), (system, machine, name)
    assert not pi.here({"FACE_PI": "off"}, "linux", "aarch64", pi4)         # the general flow there, to compare
    assert pi.here({"FACE_PI": "on"}, "win32", "AMD64", "")                 # to try it elsewhere: never by itself
    assert not pi.here({"FACE_PI": "whatever"}, "win32", "AMD64", "")
    assert pi.board("/no/such/file") == ""
    # the tracker makes it only there, and its frames take the general way everywhere else
    code = (tracker.HERE / "tracker.py").read_text(encoding="utf-8")
    assert "self.pi = pi.Prints(self.faces) if pi.here() else None" in code
    assert "self.faces.detect(img, sizes=(640,)) if self.pi is None else self.pi.find(img)" in code


def test_the_pis_frames_find_what_the_general_flow_finds(monkeypatch):
    """The same faces in the same order with the same crops and qualities as face_id.Faces.detect(sizes=(640,));
    only the prints come later."""
    faces = Faces()
    faces.boxes = [(40, 60, 120, .7), (400, 80, 100, .9), (300, 300, 20, .95)]      # the last is too small to read
    img = frame(50, 200)
    general = F.Faces.detect(faces, img, sizes=(640,))
    p = pi.Prints(faces, start=False)
    own = p.find(img)
    assert len(own) == len(general) == 2
    for (row, emb, crop, q), (grow, _gemb, gcrop, gq) in zip(own, general):
        assert np.array_equal(row, grow) and np.array_equal(crop, gcrop) and q == gq and emb is None
    assert faces.embeds == 2                                     # the general flow's two; the Pi's frame made none
    for _ in range(2):
        assert p.step()
    later = p.find(img)
    for (_, emb, _, _), (_, gemb, _, _) in zip(later, general):
        assert np.array_equal(emb, gemb)                         # face_id's own print of the same crop


def test_a_frame_never_waits_for_a_print_and_carries_each_new_one_once():
    now = [100.0]
    faces = Faces()
    faces.boxes = [(40, 60, 120, .9)]
    p = pi.Prints(faces, start=False, clock=lambda: now[0])
    (row, emb, crop, q), = p.find(frame(50))
    assert emb is None and p.named(0) == ("Unknown", 0.0) and faces.embeds == 0       # new in view: no name yet
    assert p.step() and faces.embeds == 1                        # its print, made behind the frames
    now[0] += 0.1
    (_, emb, _, _), = p.find(frame(50))
    assert emb is not None                                       # the next frame carries it
    p.name(0, "Ann", 0.82)                                       # what the tracker's people called it
    now[0] += 0.1
    (_, emb, _, _), = p.find(frame(50))
    assert emb is None and p.named(0) == ("Ann", 0.82)           # once only; the name stays on the face
    assert not p.step()                                          # nothing due before `refresh`
    now[0] += pi.REFRESH
    faces.boxes = [(48, 64, 120, .9)]                            # it moved a little: the same face
    p.find(frame(90))
    assert p.step() and faces.embeds == 2                        # its next print, from its newest picture
    (_, emb, _, _), = p.find(frame(90))
    assert emb is not None and int(np.argmax(emb)) == 90 % 8 and p.named(0) == ("Ann", 0.82)
    assert p.made == 2


def test_a_new_face_comes_first_and_a_face_that_left_is_forgotten():
    now = [10.0]
    faces = Faces()
    faces.boxes = [(40, 60, 120, .9)]
    p = pi.Prints(faces, start=False, clock=lambda: now[0])
    p.find(frame(10))
    assert p.step()
    p.find(frame(10))
    p.name(0, "Ann", 0.8)
    now[0] += pi.REFRESH + 0.1                                   # Ann's next print is due, and someone walks in
    faces.boxes = [(40, 60, 120, .9), (400, 80, 100, .8)]
    found = p.find(frame(10, 20))
    assert [e is None for _, e, _, _ in found] == [True, True] and p.named(1) == ("Unknown", 0.0)
    assert p.step()
    found = p.find(frame(10, 20))
    assert found[0][1] is None and found[1][1] is not None       # the new face first, Ann after
    assert p.step() and p.find(frame(10, 20))[0][1] is not None
    # someone else where Ann was a moment ago, far enough to be another face: no name carried over
    faces.boxes = [(200, 200, 120, .9)]
    p.find(frame(10))
    assert p.named(0) == ("Unknown", 0.0)
    now[0] += pi.FORGET + 0.1                                    # nobody in view for a while
    faces.boxes = []
    assert p.find(frame(10)) == [] and p.followed == []
    assert not p.step()


def test_the_tracker_answers_a_pi_frame_without_a_print_and_the_general_flow_as_always(monkeypatch):
    """tracker.Engine.recognize: on every other computer each face has its print in every frame; on a Pi a face
    without a new print has "emb": null, which the page has always taken as "no print to add"."""
    class People:
        def who(self, emb, threshold):
            return f"person {int(np.argmax(emb))}", 0.9

    def engine(own):
        e = object.__new__(tracker.Engine)
        e.faces, e.lock, e.ident, e.people_mtime, e.recorder = Faces(), threading.Lock(), People(), None, None
        e.faces.boxes = [(40, 60, 120, .9)]
        e.pi = pi.Prints(e.faces, start=False) if own else None
        monkeypatch.setattr(tracker.Engine, "_reload", lambda self: None)
        return e
    jpeg = cv2.imencode(".jpg", frame(50))[1].tobytes()
    general = engine(False).recognize(jpeg, 0.35)
    face, = general["faces"]
    assert face["emb"] and face["name"].startswith("person ") and face["sim"] == 0.9
    e = engine(True)
    first, = e.recognize(jpeg, 0.35)["faces"]
    assert first["emb"] is None and (first["name"], first["sim"]) == ("Unknown", 0.0)
    assert first["box"] == face["box"] and first["quality"] == face["quality"]          # all else as everywhere
    assert e.pi.step()
    second, = e.recognize(jpeg, 0.35)["faces"]
    assert second["emb"] == face["emb"] and second["name"] == face["name"]
    third, = e.recognize(jpeg, 0.35)["faces"]
    assert third["emb"] is None and third["name"] == face["name"] and third["sim"] == 0.9
    # the page takes a face without a print as it always has
    page = (tracker.HERE / "tracker_ui.js").read_text(encoding="utf-8")
    assert "if (d.emb) {" in page


def test_only_a_pis_page_is_told_to_make_smaller_pictures(track):  # noqa: F811
    """The owner's choice for the Pi (2026-11-20): its page sends smaller pictures to be read and to the other
    devices. Every other computer's page hears nothing of it and keeps 960 px."""
    import json
    url, engine = track
    code, _, body = request(url, "/api/people", headers={"X-Face-Tool": "1"})
    assert code == 200 and "pi" not in json.loads(body)                       # not a Pi: no word of it
    engine.pi = object()                                                      # as tracker.Engine has on a Pi
    code, _, body = request(url, "/api/people", headers={"X-Face-Tool": "1"})
    assert json.loads(body)["pi"] == {"send": 640, "stream": 640} == pi.PAGE
    page = (tracker.HERE / "tracker_ui.js").read_text(encoding="utf-8")
    assert "const SEND_MAX = 960;" in page and "let sendMax = SEND_MAX, streamMax = 960;" in page
    assert "if (r.pi) { if (r.pi.send > 0) sendMax = +r.pi.send; if (r.pi.stream > 0) streamMax = +r.pi.stream; }" in page
    assert "Math.min(1, sendMax / Math.max(vw, vh))" in page and "Math.min(1, streamMax / video.videoWidth)" in page
    assert page.count("sendMax") == 3 and page.count("streamMax") == 3        # set nowhere else
