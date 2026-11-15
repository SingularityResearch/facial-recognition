"""Music as a signal (music.py), its log (musiclog.py) and the owner's Spotify library (spotify.py): what counts as
a song playing, the measures of a sound whose beat and key are known, a person's usual music from their liked
songs, what is logged when, and the permission Spotify gives, against a stand-in for Spotify on this computer."""
import http.server
import json
import threading
import time
import urllib.parse

import pytest

import music


def test_a_players_title_gives_artist_and_song_only_while_a_song_plays():
    assert music.split_title("Mad Season - Wake Up") == ("Mad Season", "Wake Up")
    assert music.split_title("Artist - Song - Remastered 2011") == ("Artist", "Song - Remastered 2011")
    for nothing in ("Spotify Premium", "Spotify Free", "Spotify", "", None, "one word", " - "):
        assert music.split_title(nothing) is None
    assert music.now_playing("NoSuchSystem") is None or isinstance(music.now_playing("NoSuchSystem"), dict)


def tone(seconds=12, beats=2.0, notes=(220.0, 261.63, 329.63), level=0.2):
    """A chord that pulses `beats` times a second (A minor at 120 a minute by default)."""
    np = pytest.importorskip("numpy")
    t = np.arange(int(music.RATE * seconds)) / music.RATE
    chord = sum(np.sin(2 * np.pi * f * t) for f in notes) / len(notes)
    beat = (np.modf(t * beats)[0] < 0.15).astype(np.float32)
    return (level * chord * (0.3 + 0.7 * beat)).astype(np.float32)


def test_a_sound_is_measured_as_numbers_and_silence_as_nothing():
    np = pytest.importorskip("numpy")
    m = music.measure(tone())
    assert 110 <= m["tempo"] <= 126 and m["pulse"] > 0.3                 # 120 beats a minute, clearly
    assert (m["key"], m["mode"]) == ("A", "minor") and m["mode_sure"] > 0.5
    assert -32 < m["level"] < -24 and m["bright"] < 600 and m["bass"] < 0.05 and m["dynamics"] > 3
    major = music.measure(tone(notes=(261.63, 329.63, 392.0), beats=1.5))       # C major at 90 a minute
    assert (major["key"], major["mode"]) == ("C", "major") and 84 <= major["tempo"] <= 96
    assert music.measure(np.zeros(music.RATE * 5, np.float32)) is None         # silence
    assert music.measure(tone(seconds=1)) is None                             # too short to say anything
    assert set(m) == {"level", "dynamics", "tempo", "pulse", "bright", "bass", "change", "key", "mode", "mode_sure"}
    assert all(v is None or isinstance(v, (int, float, str)) for v in m.values())     # numbers and short text only


def test_several_listenings_make_one_set_of_numbers_and_an_output_counts_before_a_microphone():
    a = {"level": -20.0, "dynamics": 8, "tempo": 100, "pulse": 0.5, "bright": 1500, "bass": 0.1, "change": 0.1,
         "key": "A", "mode": "minor", "mode_sure": 0.8}
    b = {**a, "tempo": 104, "level": -22.0}
    c = {**a, "tempo": 180, "key": "C", "mode": "major"}
    got = music.combine([a, b, c, None])
    assert got["heard"] == 3 and got["tempo"] == 104 and (got["key"], got["mode"]) == ("A", "minor")
    assert music.combine([None, None]) is None and music.combine([]) is None
    heard = [{"name": "room microphone", "kind": "input", "default": True, "heard": {**a, "level": -30.0}},
             {"name": "headset", "kind": "output", "default": True, "heard": {**a, "level": -45.0}},
             {"name": "display", "kind": "output", "default": False, "heard": None}]
    assert music.best(heard)["name"] == "headset"                 # the song as played, though the room is louder
    assert music.best(heard[:1])["name"] == "room microphone" and music.best(heard[2:]) is None


def test_without_the_sound_library_nothing_is_listened_to(monkeypatch):
    import builtins
    real = builtins.__import__

    def no_soundcard(name, *a, **k):
        if name == "soundcard":
            raise ImportError("no soundcard here")
        return real(name, *a, **k)
    monkeypatch.setattr(builtins, "__import__", no_soundcard)
    assert music.interfaces() == [] and music.listen_all(1) == []
    assert "sound library can't be used" in music.WHY[0]
    assert music.record({"id": "x", "kind": "output"}, 1) is None


def test_a_computer_without_speakers_still_has_its_microphones(monkeypatch):
    """Each kind of interface is listed by itself: one that can't be listed (no output at all, no default one)
    doesn't take the others with it. A NAS with a display and a webcam listed nothing."""
    import sys
    import types

    def none():
        raise RuntimeError("no default device")

    class Dev:
        def __init__(self, id_, name):
            self.id, self.name = id_, name
    fake = types.SimpleNamespace(default_speaker=none, all_speakers=none, default_microphone=none,
                                 all_microphones=lambda include_loopback=False: [Dev("m1", "Webcam microphone")])
    monkeypatch.setitem(sys.modules, "soundcard", fake)
    assert music.interfaces() == [{"id": "m1", "name": "Webcam microphone", "kind": "input", "default": False}]
    assert len(music.WHY) == 1 and "outputs couldn't be listed" in music.WHY[0]
    fake.all_microphones = lambda include_loopback=False: []
    fake.all_speakers = lambda: []
    assert music.interfaces() == [] and "no audio interface" in music.WHY[0]


SONGS = [{"id": f"t{k}", "name": f"song {k}", "artists": [{"id": a, "name": n}], "album": "al", "released": rel,
          "added": add, "ms": 240000, "explicit": k == 0, "popularity": pop}
         for k, (a, n, rel, add, pop) in enumerate([
             ("a1", "Alice In Chains", "1992-09-29", "2019-03-01T10:00:00Z", 70),
             ("a1", "Alice In Chains", "1994-01-25", "2020-03-01T10:00:00Z", 62),
             ("a1", "Alice In Chains", "1995", "2021-03-01T10:00:00Z", 20),
             ("a2", "Mad Season", "1995-03-14", "2021-04-01T10:00:00Z", 55),
             ("a2", "Mad Season", "1995-03-14", "2021-04-02T10:00:00Z", 48),
             ("a3", "Nina Simone", "1965-06-01", "2022-01-01T10:00:00Z", 66)])]
GENRES = {"a1": ["grunge", "alternative metal"], "a2": ["grunge"], "a3": ["jazz", "soul"]}


def test_a_persons_usual_music_is_counts_shares_and_short_names():
    p = music.picture(SONGS, GENRES, now=0)
    assert (p["songs"], p["artists"], p["made"]) == (6, 3, "1970-01-01T00:00:00Z")
    assert p["top_artists"][0] == {"name": "Alice In Chains", "songs": 3, "share": 0.5}
    assert p["by_artist"] == {"Alice In Chains": 3, "Mad Season": 2}                 # more than one liked song
    assert p["genres"][0]["name"] == "grunge" and p["genres"][0]["share"] == round(5 / 6, 4)
    assert [d["name"] for d in p["decades"]] == ["1960s", "1990s"] and p["year"]["median"] == 1994
    assert (p["year"]["oldest"], p["year"]["newest"]) == (1965, 1995)
    assert p["added"] == [{"year": "2019", "songs": 1}, {"year": "2020", "songs": 1}, {"year": "2021", "songs": 3},
                          {"year": "2022", "songs": 1}]
    assert 0 < p["spread"] < 1 and p["explicit_share"] == round(1 / 6, 4) and p["popularity"]["median"] == 58
    assert p["popularity"]["little_known_share"] == round(1 / 6, 4) and p["minutes"]["median"] == 4.0
    json.dumps(p)                                                                    # plain data throughout
    assert music.picture([], {}) == {"songs": 0}
    assert music.picture(SONGS)["genres"] == [] and music.picture(SONGS)["songs_with_genres"] == 0   # Spotify gave none


def test_a_song_is_set_against_the_persons_usual_music():
    p = music.picture(SONGS, GENRES)
    one = music.place("mad season", "Wake Up", p)
    assert one["artist_rank"] == 2 and one["artist_songs"] == 2 and one["usual"] == "one of their artists"
    assert music.place("Soundgarden", "Fell On Black Days", p, ["grunge"])["usual"] == "their kind of music"
    assert music.place("Some Polka Band", "x", p, ["polka"])["usual"] == "not their usual"
    assert music.place("Unknown Artist", "x", p)["usual"] == "not their usual"        # nothing known about them
    assert music.place("x", "y", {})["usual"] is None                               # no picture: no verdict


# --- the log ----------------------------------------------------------------------------------------------------------

@pytest.fixture
def home(tmp_path, monkeypatch):
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path)
    return tmp_path


def test_only_a_named_persons_songs_are_kept_and_only_as_numbers_and_short_text(home, monkeypatch):
    import musiclog
    now = int(time.time() * 1000)
    row = {"t": now, "name": "Ann", "artist": "Mad Season", "song": "Wake Up", "seconds": 240.04,
           "data": {"app": "Spotify", "via": "output", "heard": {"tempo": 72.0, "mode": "minor"},
                    "interfaces": {"headset": {"kind": "output", "tempo": 72.0}}}}
    assert musiclog.check(row)["seconds"] == 240.0
    assert musiclog.check({**row, "artist": ""})["artist"] == ""      # music heard in the room: nobody named the artist
    for bad in ({**row, "name": "Unknown"}, {**row, "name": "#ignore"}, {**row, "song": ""}, {**row, "song": None},
                {**row, "t": now - 3 * 86_400_000}, {**row, "seconds": -1}, {**row, "data": {"sound": [0.1] * 5000}},
                {**row, "data": {"list": [1, 2]}}, "not a row"):
        with pytest.raises(ValueError):
            musiclog.check(bad)
    assert musiclog.add(row) and musiclog.add({**row, "t": now + 1, "name": "Bo"})
    assert [r["name"] for r in musiclog.waiting()] == ["Ann", "Bo"] and len(musiclog.waiting("Ann")) == 1
    assert musiclog.forget("Ann") == 1 and [r["name"] for r in musiclog.waiting()] == ["Bo"]


def test_nothing_is_logged_until_the_owner_switches_it_on_for_a_person(home, monkeypatch):
    import devicesettings
    import facedb
    import musiclog
    monkeypatch.setattr(facedb, "configured", lambda: True)
    assert not musiclog.enabled() and musiclog.who() == "" and musiclog.listen_kinds() == ("output", "input")
    devicesettings.update("label", {musiclog.OPTION: "1"})
    assert not musiclog.enabled()                                    # nobody named whose music it is
    devicesettings.update("label", {musiclog.WHO: "Ann", musiclog.LISTEN: "outputs"})
    assert musiclog.enabled() and musiclog.who() == "Ann" and musiclog.listen_kinds() == ("output",)
    devicesettings.update("label", {musiclog.LISTEN: "none"})
    assert musiclog.listen_kinds() == ()
    monkeypatch.setattr(facedb, "configured", lambda: False)
    assert not musiclog.enabled()                                    # nowhere to keep it


def test_a_song_is_logged_when_it_ends_with_what_each_interface_heard(home, monkeypatch):
    import devicesettings
    import facedb
    import musiclog
    monkeypatch.setattr(facedb, "configured", lambda: True)
    devicesettings.update("label", {musiclog.OPTION: "1", musiclog.WHO: "Ann"})
    musiclog.set_picture("Ann", music.picture(SONGS, GENRES))
    playing = {"now": None}
    numbers = {"level": -20.0, "dynamics": 8.0, "tempo": 72.0, "pulse": 0.5, "bright": 1500.0, "bass": 0.1,
               "change": 0.1, "key": "A", "mode": "minor", "mode_sure": 0.8}
    asked = []

    def listen_all(seconds, kinds):
        asked.append(kinds)
        return [{"name": "Speakers (a headset with a rather long name indeed)", "kind": "output", "default": True, "heard": numbers},
                {"name": "Display", "kind": "output", "default": False, "heard": None},
                {"name": "Room microphone", "kind": "input", "default": True, "heard": {**numbers, "level": -50.0}}]
    monkeypatch.setattr(music, "now_playing", lambda: playing["now"])
    monkeypatch.setattr(music, "listen_all", listen_all)
    lis, t0 = musiclog.Listener(), time.time()
    lis.step(t0)                                                     # nothing plays: listened to, nothing kept
    assert len(asked) == 1 and lis.song is None and lis.last["interfaces"][0]["kind"] == "output"
    playing["now"] = {"artist": "Mad Season", "song": "Wake Up", "app": "Spotify"}
    for k in range(0, 100, 5):
        lis.step(t0 + 10 + k)                                        # the song plays for 95 seconds
    assert len(asked) == 5 and asked[0] == ("output", "input")       # every half minute, every interface: 4 in the song
    playing["now"] = {"artist": "Skipped", "song": "Past", "app": "Spotify"}
    lis.step(t0 + 110)
    playing["now"] = None
    lis.step(t0 + 115)                                               # seen for 5 seconds: skipped past, not logged
    rows = musiclog.waiting()
    assert [(r["name"], r["artist"], r["song"]) for r in rows] == [("Ann", "Mad Season", "Wake Up")]
    r = rows[0]
    assert r["seconds"] == 95.0 and r["t"] == int((t0 + 10) * 1000)
    d = r["data"]
    assert d["app"] == "Spotify" and d["via"] == "output" and d["heard"]["tempo"] == 72.0 and d["heard"]["heard"] == 4
    assert set(d["interfaces"]) == {"Speakers (a headset with a rather long n", "Room microphone"}     # the silent one isn't there
    assert d["interfaces"]["Room microphone"]["kind"] == "input" and d["on"].startswith("Speakers")
    assert d["interfaces"]["Room microphone"]["music"]["tempo"] == 72.0 and d["interfaces"]["Room microphone"]["music"]["heard"] == 4
    assert d["place"]["usual"] == "one of their artists" and d["place"]["artist_rank"] == 2
    devicesettings.update("label", {musiclog.OPTION: "0"})           # switched off while a song plays: it isn't kept
    playing["now"] = {"artist": "Mad Season", "song": "River Of Deceit", "app": "Spotify"}
    lis.step(t0 + 200)
    assert lis.song is None and len(musiclog.waiting()) == 1


def test_music_heard_in_the_room_is_logged_without_a_player_naming_it(home, monkeypatch):
    """No player shows a song, and a microphone hears music twice in a row: a row "music in the room", with what
    each interface heard in a word, the families of sound and the background's numbers. It ends when the music does."""
    import devicesettings
    import facedb
    import musiclog
    monkeypatch.setattr(facedb, "configured", lambda: True)
    devicesettings.update("label", {musiclog.OPTION: "1", musiclog.WHO: "Ann"})
    numbers = {"level": -40.0, "dynamics": 8.0, "tempo": 96.0, "pulse": 0.4, "bright": 1200.0, "bass": 0.1,
               "change": 0.1, "key": "C", "mode": "major", "mode_sure": 0.6}
    amb = {"level": -40.0, "floor": -55.0, "events": 20.0, "steady": 0.2, "flat": 0.1, "hum": 1.0, "low": 0.1,
           "high": 0.05, "speechlike": 0.3}
    room = {"what": "music"}

    def listen_all(seconds, kinds):
        m = room["what"] == "music"
        return [{"name": "Headset", "kind": "output", "default": True, "heard": None, "sounds": None, "families": None,
                 "ambient": None, "what": "quiet", "sure": 1.0, "background": ""},
                {"name": "Room microphone", "kind": "input", "default": True, "heard": numbers if m else None,
                 "sounds": {"Music": 0.7} if m else {"Mechanical fan": 0.5},
                 "families": {"music": 0.7} if m else {"steady background noise": 0.5}, "ambient": amb,
                 "what": room["what"], "sure": 0.7, "background": "" if m else "broadband, fan-like"}]
    monkeypatch.setattr(music, "now_playing", lambda: None)
    monkeypatch.setattr(music, "listen_all", listen_all)
    lis, t0 = musiclog.Listener(), time.time()
    lis.step(t0)
    assert lis.song is None                                          # heard once: could be a passing sound
    for k in (30, 60, 90):
        lis.step(t0 + k)
    assert lis.song["song"] == musiclog.ROOM_SONG and lis.song["t0"] == t0
    room["what"] = "steady background noise"
    lis.step(t0 + 120)                                               # the music stopped
    assert lis.song is None
    (r,) = musiclog.waiting()
    assert (r["name"], r["artist"], r["song"], r["seconds"]) == ("Ann", "", "music in the room", 90.0)
    d = r["data"]
    assert d["app"] == "room" and d["via"] == "input" and d["heard"]["tempo"] == 96.0 and "place" not in d
    mic = d["interfaces"]["Room microphone"]
    assert mic["what"] == "music" and mic["families"] == {"music": 0.7} and mic["ambient"]["floor"] == -55.0
    assert "Headset" not in d["interfaces"]                         # quiet: not in the row
    st = musiclog.status()
    assert st["today"] == 1 and st["earlier"] == 0 and "waiting" not in st      # today's songs, sent or not
    old = musiclog.folder() / "2020-01-01.jsonl"                     # a past day's file is there only while unsent
    old.write_text(json.dumps({"t": 1, "name": "Ann", "artist": "x", "song": "y"}) + "\n", "utf-8")
    assert musiclog.status()["earlier"] == 1


def test_a_background_is_named_by_family_and_described_by_its_numbers():
    np = pytest.importorskip("numpy")
    fam = music.families({"Waterfall": 0.34, "Water": 0.33, "Pink noise": 0.07, "Speech": 0.05, "Bird song": 0.04,
                          "Music": 0.03, "Typing": 0.02, "Zebra crossing": 0.02})
    assert fam["steady background noise"] == 0.74 and fam["speech"] == 0.05 and fam["animal sounds"] == 0.04
    assert fam["music"] == 0.03 and fam["activity"] == 0.02 and fam["other sounds"] == 0.02
    assert list(fam)[0] == "steady background noise" and music.families(None) == {}
    rng = np.random.default_rng(3)
    t = np.arange(music.RATE * 8) / music.RATE
    fan = (0.02 * rng.standard_normal(len(t))).astype(np.float32)                      # steady hiss, no events
    a = music.ambient(fan + 0.3)                                                      # with an input's offset
    assert -36 < a["level"] < -32 and abs(a["floor"] - a["level"]) < 2 and a["events"] < 3 and a["steady"] > 0.8
    assert a["flat"] > 0.5 and a["hum"] < 6
    assert music.what(None, a) == ("steady background noise", 0.5)                    # no model: only how it behaves
    assert music.what({"Waterfall": 0.4, "Pink noise": 0.1}, a)[0] == "steady background noise"
    assert music.background(a) in ("hiss", "broadband, fan-like")
    hum = (0.05 * np.sin(2 * np.pi * 60 * t) + 0.02 * np.sin(2 * np.pi * 120 * t) + 0.01 * np.sin(2 * np.pi * 180 * t)
           + 0.0005 * rng.standard_normal(len(t))).astype(np.float32)
    h = music.ambient(hum)
    assert h["hum"] > 15 and h["low"] > 0.8 and "mains hum" in music.background(h)
    talk = (0.05 * rng.standard_normal(len(t)) * (np.sin(2 * np.pi * 4 * t) > 0.3)).astype(np.float32)   # comes and goes
    s = music.ambient(talk)
    assert s["steady"] < 0.5 and s["events"] > 20 and s["floor"] < s["level"] - 10
    assert music.what({"Speech": 0.6, "Music": 0.1}, s) == ("speech", 0.6)
    assert music.what({"Music": 0.5, "Speech": 0.3}, s) == ("music", 0.5)
    assert music.what({"Silence": 0.9}, s)[0] == "quiet" and music.what(None, None) == ("quiet", 1.0)
    assert music.what({"Typing": 0.5}, s) == ("activity", 0.5)
    assert music.ambient(np.zeros(music.RATE * 4, np.float32)) is None and music.ambient(fan[:100]) is None
    loud = dict(a, level=-4.0, steady=0.9)
    assert "nothing plugged in" in music.background(loud)
    assert music.sound_model() is None or music.sound_model().name == "yamnet.tflite"


def test_the_usual_music_is_a_person_signal_the_pages_never_get(home):
    import devicesettings
    import musiclog
    musiclog.set_picture("Ann", {"songs": 3})
    devicesettings.put_signal("usual", "Ann", {"n": 1})
    assert "music" not in devicesettings.signals() and devicesettings.signals(every=True)["music"] == {"Ann": {"songs": 3}}
    assert set(devicesettings.page_state("track")["signals"]) == {"ft-usual", "ft-baselines"}
    assert devicesettings.of("Ann") == {"usual": {"n": 1}, "music": {"songs": 3}}
    assert devicesettings.forget("Ann") == 2 and musiclog.picture_of("Ann") is None
    for bad in (("music", "#ignore", {}), ("music", "", {}), ("nonsense", "Ann", {}), ("music", "Ann", [1])):
        with pytest.raises(ValueError):
            devicesettings.put_signal(*bad)


# --- Spotify, with a stand-in on this computer -----------------------------------------------------------------------

@pytest.fixture
def spotify_stand_in(home, monkeypatch):
    """A server on this computer that answers as Spotify's token and library addresses do."""
    import spotify
    seen = {"tokens": [], "asked": []}
    liked = [{"added_at": f"2021-01-{k % 28 + 1:02d}T00:00:00Z",
              "track": {"id": f"t{k}", "type": "track", "name": f"song {k}", "duration_ms": 200000, "explicit": False,
                        "popularity": 40, "artists": [{"id": f"a{k % 3}", "name": f"artist {k % 3}"}],
                        "album": {"name": "album", "release_date": "1995-03-14"}}} for k in range(120)]
    liked.insert(5, {"added_at": "2021-01-01T00:00:00Z", "track": None})          # a song Spotify no longer has

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def _send(self, obj, code=200):
            body = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            form = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
            seen["tokens"].append(form)
            if form.get("grant_type") == "authorization_code" and form.get("code") != "the-code":
                return self._send({"error": "invalid_grant", "error_description": "Invalid authorization code"}, 400)
            self._send({"access_token": f"access-{len(seen['tokens'])}", "refresh_token": "refresh-1", "expires_in": 3600})

        def do_GET(self):
            u = urllib.parse.urlsplit(self.path)
            q = dict(urllib.parse.parse_qsl(u.query))
            seen["asked"].append((u.path, self.headers.get("Authorization")))
            if u.path == "/v1/elsewhere":                           # where a redirect would lead: must never be asked
                seen["leaked"] = self.headers.get("Authorization")
                return self._send({})
            if u.path == "/v1/moved":
                self.send_response(302)
                self.send_header("Location", f"http://127.0.0.1:{self.server.server_address[1]}/v1/elsewhere")
                self.send_header("Content-Length", "0")
                return self.end_headers()
            if not (self.headers.get("Authorization") or "").startswith("Bearer access-"):
                return self._send({"error": "no token"}, 401)
            if u.path == "/v1/me/tracks":
                o, n = int(q["offset"]), int(q["limit"])
                return self._send({"total": len(liked), "items": liked[o:o + n]})
            if u.path == "/v1/artists":
                return self._send({"artists": [{"id": i, "genres": ["grunge"] if i == "a0" else []} for i in q["ids"].split(",")]})
            self._send({}, 404)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    monkeypatch.setattr(spotify, "TOKEN", base + "/api/token")
    monkeypatch.setattr(spotify, "API", base + "/v1")
    monkeypatch.setattr(spotify, "CONF", home / "spotify.json")
    yield seen
    srv.shutdown()


CLIENT = "0123456789abcdef0123456789abcdef"


def test_the_permission_is_asked_with_a_one_time_code_and_the_tokens_kept_protected(spotify_stand_in, home):
    import spotify
    assert spotify.status() == {"connected": False, "client_id": "", "since": None}
    for bad in ("", "too short", "g" * 32, None):
        with pytest.raises(ValueError):
            spotify.begin(bad)
    url, verifier, state = spotify.begin(CLIENT)
    q = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
    assert url.startswith("https://accounts.spotify.com/authorize?") and q["scope"] == "user-library-read"   # read only
    assert q["code_challenge_method"] == "S256" and q["state"] == state and q["redirect_uri"] == spotify.redirect_uri()
    assert spotify.redirect_uri().startswith("http://127.0.0.1:") and "client_secret" not in url
    with pytest.raises(OSError, match="Invalid authorization code"):
        spotify.finish("a-wrong-code", verifier, CLIENT)
    assert not spotify.status()["connected"]
    st = spotify.finish("the-code", verifier, CLIENT)
    assert st["connected"] and st["client_id"] == CLIENT and st["since"]
    sent = spotify_stand_in["tokens"][-1]
    assert sent["code_verifier"] == verifier and sent["client_id"] == CLIENT and "client_secret" not in sent
    kept = (home / "spotify.json").read_text("utf-8")
    assert "refresh-1" not in kept and "access-" not in kept             # protected, never as they are
    spotify.disconnect()
    assert spotify.status() == {"connected": False, "client_id": CLIENT, "since": None}
    with pytest.raises(spotify.NotConnected):
        spotify.liked()


def test_the_one_answer_from_spotify_is_taken_and_nothing_else(spotify_stand_in):
    import socket
    import urllib.request

    import spotify
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    got = {}

    def wait(state, timeout):
        try:
            got["code"] = spotify.wait_for_code(state, timeout=timeout, port=port)
        except OSError as e:
            got["error"] = str(e)
    t = threading.Thread(target=wait, args=("the-state", 10))
    t.start()
    time.sleep(0.3)

    def ask(path):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=5) as r:
                return r.status, r.read().decode()
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode()
    assert ask("/callback?code=stolen&state=someone-elses")[0] == 404      # not this request's state: ignored
    assert ask("/other")[0] == 404 and "code" not in got
    code, page = ask("/callback?code=abc123&state=the-state")
    assert code == 200 and "connected" in page
    t.join(5)
    assert got == {"code": "abc123"}
    t = threading.Thread(target=wait, args=("s2", 10))                     # the owner said no on Spotify's page
    got.clear()
    t.start()
    time.sleep(0.3)
    ask("/callback?error=access_denied&state=s2")
    t.join(5)
    assert "access_denied" in got["error"]
    got.clear()
    wait("s3", 1.2)                                                        # nobody answered
    assert "in time" in got["error"]


def test_every_liked_song_is_read_page_by_page_as_short_text_and_numbers(spotify_stand_in):
    import spotify
    _url, verifier, _state = spotify.begin(CLIENT)
    spotify.finish("the-code", verifier, CLIENT)
    seen = []
    songs = spotify.liked(lambda done, total: seen.append((done, total)))
    assert len(songs) == 120 and seen[-1] == (121, 121) and len(seen) == 3            # three pages of 50
    assert songs[0] == {"id": "t0", "name": "song 0", "artists": [{"id": "a0", "name": "artist 0"}], "album": "album",
                        "released": "1995-03-14", "added": "2021-01-01T00:00:00Z", "ms": 200000, "explicit": False,
                        "popularity": 40}
    g = spotify.genres({a["id"] for s in songs for a in s["artists"]})
    assert g == {"a0": ["grunge"], "a1": [], "a2": []}
    assert all(auth and auth.startswith("Bearer access-") for _p, auth in spotify_stand_in["asked"])
    with pytest.raises(OSError, match="302"):                        # a redirect isn't followed: the token would go along
        spotify._get("/moved")
    assert "leaked" not in spotify_stand_in
    p = music.picture(songs, g)
    assert p["songs"] == 120 and p["artists"] == 3 and p["genres"][0]["name"] == "grunge"


def test_reading_the_liked_songs_makes_the_persons_usual_music(spotify_stand_in, home, monkeypatch):
    import devicesettings
    import musiclog
    import spotify
    monkeypatch.setattr(musiclog, "JOB", {"state": "idle", "text": ""})
    with pytest.raises(ValueError, match="whose music"):
        musiclog.read_liked()
    devicesettings.update("label", {musiclog.WHO: "Ann"})
    with pytest.raises(ValueError, match="Connect Spotify"):
        musiclog.read_liked()
    _url, verifier, _state = spotify.begin(CLIENT)
    spotify.finish("the-code", verifier, CLIENT)
    musiclog.read_liked()
    for _ in range(100):
        if musiclog.JOB["state"] in ("done", "failed"):
            break
        time.sleep(0.05)
    assert musiclog.JOB["state"] == "done" and "120 liked songs" in musiclog.JOB["text"]
    pic = musiclog.picture_of("Ann")
    assert pic["songs"] == 120 and pic["source"] == "Spotify liked songs"
    st = musiclog.status()
    assert st["who"] == "Ann" and st["picture"]["songs"] == 120 and st["spotify"]["connected"] and "by_artist" not in st["picture"]
    assert not st["logging"] and st["on"] is False                       # reading the songs doesn't switch the log on
