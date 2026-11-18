"""
musiclog.py - the music log: which song played on this device, when, for whom, and how it sounded on each of the
device's audio interfaces, kept in the shared database (table music_log, db/012) for reports later, beside the
expression log. And each person's usual music (music.picture), kept as a person signal of kind "music"
(devicesettings.py), so it is shared, exported and forgotten like the person's other normals.

Off until the owner switches it on (Settings: "Keep a log of the music that plays", fl-music), and only with a
shared database to keep it in and a person named whose music this device's is (fl-musicWho). Then a thread here
looks every few seconds at what the music app shows as playing (music.now_playing: nothing is asked of the
internet) and, while a song plays, listens for a few seconds every half minute to every audio interface at once
(music.listen_all): each output (what this computer itself plays) and each microphone (what sounds in the room).
Only numbers leave the listening; the sound is never kept. Which interfaces are listened to is the owner's choice
(fl-musicListen: all | outputs | none).

While the log is on, the listening goes on whether a player shows a song or not, so Settings can say what each
interface hears now ("quiet", "speech", "steady background noise: hiss", "music"...; music.what, from the
sound-recognition model's kinds of sound). Music an interface hears twice in a row with no player naming a song is
logged too, as "music in the room" with no artist.

One row, written when the song ends or changes:
  {"t": epoch ms it was first seen playing, "name", "artist" ("" for music in the room), "song", "seconds", "data"}
  data  app (the player, or "room"); heard (the numbers taken for the song: from an output before a microphone;
        music.combine); via (output | input) and on (that interface's name); interfaces {name: {kind, what, sure,
        background, families, ambient (the background's numbers), music (a song's numbers where music sounded)}}
        for each one that wasn't quiet; place (the song against the person's usual music)

Rows wait in HOME/music/<UTC day>.jsonl until the sync has put them in the database (dbsync_home.HomePart.after_push
-> send); a past day's file goes once all of it is there. The waiting and the sending are waitlog.py's, which the
expression log uses too; what a row is, is here. persondata.py exports and forgets a person's rows and their
picture.
"""
import json
import threading
import time
from datetime import datetime, timezone

import alerts
import forgotten
import music
import waitlog

FOLDER = "music"                 # in HOME: songs waiting to be sent
OPTION, WHO, LISTEN = "fl-music", "fl-musicWho", "fl-musicListen"
LOOK_EVERY = 5.0                 # seconds between looks at what is playing
LISTEN_EVERY = 30.0              # seconds between listenings while a song plays
LISTEN_FOR = 8.0                 # seconds of one listening
MIN_SECONDS = 15.0               # a song seen for less was skipped past: not logged
MAX_DATA = 12_000                # bytes of one row's data


def _values(o):
    """A waiting song as music_log's columns after device, name and time. One without artist or song isn't sent."""
    return o["artist"], o["song"], o.get("seconds"), json.dumps(o.get("data") or {})


# The songs waiting here, and how they reach the table. "mlog" names their files in the sync's state; a line
# with no song isn't a row.
LOG = waitlog.WaitLog(FOLDER, "music_log", ("artist", "song", "seconds", "data"), _values, key="mlog", needs=("song",))


def folder():
    return LOG.folder()


def _settings():
    import devicesettings
    return devicesettings.settings("label")


def who():
    """The person whose music this device's is (the owner's setting), or ""."""
    name = (_settings().get(WHO) or "").strip()[:100]
    return "" if not name or name == "Unknown" or name.startswith("#") else name


def listen_kinds():
    """Which interfaces are listened to: every one (the default), the outputs only, or none."""
    return {"outputs": ("output",), "none": ()}.get(_settings().get(LISTEN) or "all", ("output", "input"))


def enabled():
    """Songs are logged only with a shared database to keep them in, the switch on, and a person named."""
    import facedb
    return facedb.configured() and _settings().get(OPTION) == "1" and bool(who())


def check(r, now_ms=None):
    """One row, checked; ValueError if it isn't one."""
    if not isinstance(r, dict):
        raise ValueError("a row is an object")
    name = str(r.get("name") or "").strip()[:100]
    artist, song = str(r.get("artist") or "").strip()[:200], str(r.get("song") or "").strip()[:200]
    if not name or name == "Unknown" or name.startswith("#") or not song:
        raise ValueError("a row needs a named person and a song (the artist may be unknown: music heard in the room)")
    try:
        t = int(r.get("t"))
        seconds = None if r.get("seconds") is None else round(float(r["seconds"]), 1)
    except (TypeError, ValueError):
        raise ValueError("bad row") from None
    now_ms = time.time() * 1000 if now_ms is None else now_ms
    if abs(t - now_ms) > 86_400_000 or (seconds is not None and not 0 <= seconds <= 86_400):
        raise ValueError("row time out of range")
    data = r.get("data") or {}
    if not isinstance(data, dict) or len(json.dumps(data)) > MAX_DATA:
        raise ValueError("bad row data")
    return {"t": t, "name": name, "artist": artist, "song": song, "seconds": seconds, "data": alerts.check_details(data)}


def add(row):
    """Keep one song until the sync sends it. True if it was kept (False for a forgotten person's)."""
    row = check(row)
    if row["name"] in forgotten.names():
        return False
    LOG.add([row])
    return True


def waiting(name=None):
    """Songs not sent yet (of one person, or everyone's)."""
    return LOG.waiting(name)


def forget(name):
    """Drop name's waiting songs and their picture here (persondata.forget; the table's rows and the shared picture
    go with the sync's forget). Returns how many songs went."""
    return LOG.forget(name)


def send(conn, state, device):
    """The sync: put waiting songs into the database's music_log. One row per (device, name, time), so sending a
    line twice changes nothing. Past days' files go once all of them is in the database. Returns how many were sent."""
    return LOG.send(conn, state, device)


def export_to(path, name):
    """Write name's songs to path, one JSON object a line: those in the shared database (when it can be reached)
    and those still waiting here. Returns {"songs": n, "note": why some may be missing or None}."""
    n, note = LOG.export_to(path, name)
    return {"songs": n, "note": note}


# --- a person's usual music ---------------------------------------------------------------------------------------

def set_picture(name, pic):
    """Keep name's usual music (music.picture) as their person signal of kind "music": shared with every device
    through the sync, exported and forgotten with the person."""
    import devicesettings
    devicesettings.put_signal("music", name, pic)


def picture_of(name):
    import devicesettings
    return devicesettings.signals(every=True)["music"].get(name)


# --- Spotify: the owner's permission, and reading their liked songs (Settings) -------------------------------------

JOB = {"state": "idle", "text": ""}      # idle | connecting | reading | done | failed
_job_lock = threading.Lock()


def _job(target, state, text):
    """Run one Spotify job at a time on a thread of its own; JOB says how it goes."""
    with _job_lock:
        if JOB["state"] in ("connecting", "reading"):
            raise ValueError("Spotify is being asked already")
        JOB.clear()
        JOB.update(state=state, text=text)

    def run():
        try:
            JOB.update(state="done", text=target() or "Done.")
        except Exception as e:
            JOB.update(state="failed", text=f"{str(e)[:300] or type(e).__name__}")
    threading.Thread(target=run, daemon=True, name="spotify").start()


def connect_spotify(client_id):
    """Open Spotify's page for the owner to agree, and wait for them (five minutes at most)."""
    import spotify
    client_id = spotify.check_client_id(client_id)

    def go():
        spotify.connect(client_id, say=lambda *_: None)
        return "Spotify is connected."
    _job(go, "connecting", "Waiting for you to agree on Spotify's page in your browser.")


def read_liked():
    """Read the connected account's liked songs and make the picture of the person this device's music belongs to."""
    import spotify
    name = who()
    if not name:
        raise ValueError("Say first whose music this device's is.")
    if not spotify.status()["connected"]:
        raise ValueError("Connect Spotify first.")

    def go():
        songs = spotify.liked(lambda done, total: JOB.update(text=f"Reading liked songs: {done:,} of {total:,}"))
        ids = {a["id"] for s in songs for a in s["artists"][:2]}
        JOB.update(text=f"{len(songs):,} liked songs; reading their artists' genres")
        genres = spotify.genres(ids, lambda done, total: JOB.update(text=f"Genres: {done:,} of {total:,} artists"))
        pic = music.picture(songs, genres)
        pic["source"] = "Spotify liked songs"
        set_picture(name, pic)
        return f"{name}'s usual music is made from {pic['songs']:,} liked songs by {pic.get('artists', 0):,} artists."
    _job(go, "reading", "Reading liked songs")


def status():
    """What Settings shows: the switch, whose music, what plays now, the interfaces, Spotify, the picture."""
    import spotify
    name = who()
    pic = picture_of(name) if name else None
    try:
        sp = {**spotify.status(), "redirect": spotify.redirect_uri()}
    except Exception as e:
        sp = {"connected": False, "client_id": "", "error": str(e)[:200], "redirect": ""}
    t = _listener["thread"]
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    counts = LOG.days()     # today's file holds today's songs, sent or not; an older one only while some of it isn't sent
    return {"on": _settings().get(OPTION) == "1", "who": name, "listen": _settings().get(LISTEN) or "all",
            "today": counts.get(today, 0), "earlier": sum(n for day, n in counts.items() if day < today),
            "logging": enabled(), "playing": music.now_playing(),
            "current": {k: t.song[k] for k in ("artist", "song")} | {"heard": len(t.song["heard"])} if t and t.song else None,
            # what each interface heard at the last listening, in a word (the numbers stay here)
            "room": None if not (t and t.last) else {"ago": round(time.time() - t.last["at"]), "interfaces": [
                {k: h.get(k) for k in ("name", "kind", "what", "sure", "background")} for h in t.last["interfaces"]]},
            "model": music.sound_model() is not None,
            "interfaces": [{k: i[k] for k in ("name", "kind", "default")} for i in music.interfaces()],
            "interfaces_why": "; ".join(music.WHY),
            "spotify": sp, "job": dict(JOB),
            "picture": None if not pic else {k: pic.get(k) for k in (
                "songs", "artists", "made", "source", "top_artists", "genres", "decades", "year", "popularity", "spread")}}


# --- the listening thread -------------------------------------------------------------------------------------------

ROOM_SONG = "music in the room"  # the song of a row when no player named it: an interface heard music
ROOM_AFTER = 2                   # listenings in a row that must hear music before the room counts as playing some


def _summary(h):
    """One interface's listening as the log keeps it: what it heard in a word, how sure, the families of sound,
    the background's numbers, and a song's numbers where music sounded."""
    out = {"kind": h["kind"], "what": h.get("what") or "", "sure": h.get("sure")}
    if h.get("background"):
        out["background"] = h["background"]
    if h.get("families"):
        out["families"] = dict(list(h["families"].items())[:4])
    if h.get("ambient"):
        out["ambient"] = h["ambient"]
    return out


class Listener(threading.Thread):
    """Looks at what plays, listens to every interface, and logs each song when it ends: a song a player names, or
    music an interface hears with no player naming it ("music in the room")."""

    def __init__(self):
        super().__init__(daemon=True, name="music log")
        self.song = None            # {"artist", "song", "app", "t0", "seen", "heard": [listen_all results]}
        self.listened = 0.0
        self.last = None            # the newest listening: {"at", "interfaces": listen_all's result}
        self.room = 0               # listenings in a row in which an interface heard music, no player naming a song
        self.stop_now = threading.Event()

    def _finish(self):
        s, self.song = self.song, None
        if not s or s["seen"] - s["t0"] < MIN_SECONDS or not enabled():
            return
        per, seen = {}, {}
        for one in s["heard"]:
            for h in one:
                key = h["name"][:40]
                if h.get("what") and h["what"] != "quiet":
                    seen[key] = _summary(h)                    # the newest listening's word for each interface
                if h.get("heard"):
                    per.setdefault(key, {"kind": h["kind"], "all": []})["all"].append(h["heard"])
        data = {"app": s["app"]}
        if seen or per:
            data["interfaces"] = {k: {**seen.get(k, {"kind": per[k]["kind"]} if k in per else {}),
                                      **({"music": music.combine(per[k]["all"])} if k in per else {})}
                                  for k in {**seen, **per}}
        if per:
            top = music.best([{"name": k, "kind": v["kind"], "heard": music.combine(v["all"])} for k, v in per.items()])
            if top:
                data.update(heard=top["heard"], via=top["kind"], on=top["name"])
        pic = picture_of(who())
        if pic and s["artist"]:
            data["place"] = {k: v for k, v in music.place(s["artist"], s["song"], pic).items() if k not in ("artist", "song")}
        try:
            add({"t": int(s["t0"] * 1000), "name": who(), "artist": s["artist"], "song": s["song"],
                 "seconds": s["seen"] - s["t0"], "data": data})
        except ValueError as e:
            print(f"Music log: {e}")

    def step(self, now=None):
        """One look (and, when one is due, one listening). Separate from run() so tests can drive it."""
        now = time.time() if now is None else now
        if not enabled():
            self.song, self.last, self.room = None, None, 0
            return
        playing = music.now_playing()
        named = self.song and self.song["app"] != "room"
        same = playing and named and (playing["artist"], playing["song"]) == (self.song["artist"], self.song["song"])
        if self.song and (playing or named) and not same:      # a player's song ended or changed, or a player now
            self._finish()                                     # names what the room was playing
        if playing and not same:
            self.song = {**playing, "t0": now, "seen": now, "heard": []}
            self.listened, self.room = 0.0, 0
        if self.song and self.song["app"] != "room":
            self.song["seen"] = now
        kinds = listen_kinds()
        if kinds and now - self.listened >= LISTEN_EVERY:
            self.listened = now
            heard = music.listen_all(LISTEN_FOR, kinds)
            self.last = {"at": now, "interfaces": heard}
            music_here = any(h.get("what") == "music" for h in heard)
            if self.song and self.song["app"] != "room":
                if heard and len(self.song["heard"]) < 40:
                    self.song["heard"].append(heard)
            elif music_here:                                   # no player names a song, and music sounds
                self.room += 1
                if self.song is None and self.room >= ROOM_AFTER:
                    self.song = {"artist": "", "song": ROOM_SONG, "app": "room", "t0": now - LISTEN_EVERY * (self.room - 1),
                                 "seen": now, "heard": []}
                if self.song:
                    self.song["seen"] = now
                    if len(self.song["heard"]) < 40:
                        self.song["heard"].append(heard)
            else:
                self.room = 0
                if self.song:                                  # the room's music stopped
                    self._finish()

    def run(self):
        while not self.stop_now.wait(LOOK_EVERY):
            try:
                self.step()
            except Exception as e:                   # never let one bad look end the log
                print(f"Music log: {type(e).__name__}: {str(e)[:200]}")


_listener = {"thread": None}


def start():
    """Start the listening thread once (the labeling tool does, at its start). It does nothing until the owner
    switches the log on."""
    if _listener["thread"] is None:
        _listener["thread"] = Listener()
        _listener["thread"].start()
    return _listener["thread"]
