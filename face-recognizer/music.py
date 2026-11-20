"""
music.py - music as one more signal about a person: what is playing, how it sounds, and the person's usual music.

Three parts, each working without the others:
  now_playing()   the song a music app on this computer shows as playing (Spotify: its window title on Windows,
                  its own answer on macOS and Linux). Nothing is asked of the internet. The app shows the song
                  even when the sound comes out of another device (a phone, a speaker).
  listen_all()    every audio interface of this computer listened to at once and measured one by one: each output
                  (what the computer itself plays through its speakers, a headset, a display) and each input
                  (what a microphone hears in the room, from any device). For each: how loud, how much it
                  changes, its tempo, how bright or bassy it is, major or minor. Numbers only: the sound is
                  never kept, like the tracker's microphone. Needs the `soundcard` library (optional): nothing
                  is heard without it. A Mac can't hear its own outputs, so there only the inputs count.
  picture()       a person's usual music from their liked songs (spotify.py): artists, genres, decades, how
                  the collection grew. Short text and numbers.
  place()         one song against that picture: usual for this person, or not.

A measure describes the sound ("slow, dark, in a minor key"), never what the listener feels: the same honesty as
"looks", not "feels", for faces. What a person's music says about their state is a hypothesis until it is measured
against their own reports (tools/expr_eval.py's feeling reports).
"""
import collections
import math
import platform
import shutil
import statistics
import subprocess
import time

RATE = 22050                     # samples a second the sound is measured at
SILENT = -65.0                   # dBFS: quieter than this, nothing is playing here
PAUSED = ("spotify", "spotify free", "spotify premium", "")     # Spotify's window title when nothing plays
MAJOR = (6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88)    # Krumhansl's key profiles
MINOR = (6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17)
NOTES = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")


# --- what is playing ------------------------------------------------------------------------------------------------

def split_title(title):
    """("artist", "song") from a player's "Artist - Song" (the song may have dashes of its own), or None."""
    title = (title or "").strip()
    if title.lower() in PAUSED or " - " not in title:
        return None
    artist, _, song = title.partition(" - ")
    artist, song = artist.strip()[:200], song.strip()[:200]
    return (artist, song) if artist and song else None


def _spotify_window_title():
    """The title of the Spotify app's window on Windows ("Artist - Song" while a song plays), or None."""
    import ctypes
    from ctypes import wintypes as w
    user, kernel = ctypes.windll.user32, ctypes.windll.kernel32
    kernel.OpenProcess.restype = w.HANDLE
    found = []

    def each(hwnd, _):
        n = user.GetWindowTextLengthW(hwnd)
        if n <= 0 or not user.IsWindowVisible(hwnd) and not user.IsIconic(hwnd):
            return True
        pid = w.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        h = kernel.OpenProcess(0x1000, False, pid.value)             # PROCESS_QUERY_LIMITED_INFORMATION
        if not h:
            return True
        try:
            buf, size = ctypes.create_unicode_buffer(520), w.DWORD(520)
            if kernel.QueryFullProcessImageNameW(w.HANDLE(h), 0, buf, ctypes.byref(size)) \
                    and buf.value.lower().endswith("\\spotify.exe"):
                text = ctypes.create_unicode_buffer(n + 1)
                user.GetWindowTextW(hwnd, text, n + 1)
                found.append(text.value)
        finally:
            kernel.CloseHandle(w.HANDLE(h))
        return True
    proc = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)(each)
    user.EnumWindows(proc, 0)
    return next((t for t in found if split_title(t)), None)


def _run(cmd, timeout=3):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def now_playing(system=None):
    """{"artist", "song", "app"} of the song a music app here shows as playing, or None."""
    system = system or platform.system()
    try:
        if system == "Windows":
            got = split_title(_spotify_window_title())
        elif system == "Darwin":                 # only ask a Spotify that is open: asking would start it
            if not _run(["pgrep", "-x", "Spotify"]):
                return None
            out = _run(["osascript", "-e", 'tell application "Spotify" to if player state is playing then '
                        '(artist of current track) & " - " & (name of current track)'])
            got = split_title(out)
        else:
            if not shutil.which("playerctl"):
                return None
            out = _run(["playerctl", "-p", "spotify", "metadata", "--format", "{{status}}\t{{artist}} - {{title}}"])
            status, _, title = out.partition("\t")
            got = split_title(title) if status == "Playing" else None
    except Exception:
        return None
    return {"artist": got[0], "song": got[1], "app": "Spotify"} if got else None


# --- how it sounds --------------------------------------------------------------------------------------------------

WHY = []                         # why interfaces() found none, or not all (for Settings)


def interfaces():
    """Every audio interface of this computer that can be listened to: [{id, name, kind, default}].
    kind "output": what the computer itself plays through that interface (speakers, a headset, a display's
    speakers), heard exactly as played. kind "input": what a microphone hears, the room and whatever plays in it
    from any device. [] without the `soundcard` library. A Mac has no way to hear its own outputs: inputs only."""
    import warnings
    WHY.clear()
    try:
        import soundcard as sc
    except Exception as e:                       # not installed, or (Linux) no PulseAudio library to talk to
        WHY.append(f"the sound library can't be used here ({type(e).__name__})")
        return []
    found = []
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        # each step by itself: a computer without speakers (or without a default one) still has its microphones,
        # and one step failing took every interface with it (a NAS with a display and a webcam listed none)
        for kind, every, default in (("output", sc.all_speakers, sc.default_speaker),
                                     ("input", lambda: sc.all_microphones(include_loopback=False), sc.default_microphone)):
            try:
                first = getattr(default(), "id", None)
            except Exception:
                first = None
            try:
                found += [{"id": str(d.id), "name": str(d.name)[:120], "kind": kind, "default": d.id == first}
                          for d in every()]
            except Exception as e:
                WHY.append(f"the {kind}s couldn't be listed ({type(e).__name__}: {str(e)[:80]})")
    if not found and not WHY:
        WHY.append("this computer has no audio interface the app can see")
    return found


def record(interface, seconds=10.0):
    """What one interface (an entry of interfaces()) plays or hears for `seconds`, as one channel of numbers at
    RATE, or None when it can't be listened to. Kept in memory only, never written anywhere."""
    try:
        import warnings

        import numpy as np
        import soundcard as sc
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            mic = sc.get_microphone(id=interface["id"], include_loopback=interface["kind"] == "output")
            with mic.recorder(samplerate=RATE, channels=min(2, max(1, mic.channels))) as rec:
                x = rec.record(numframes=int(RATE * seconds))
        x = np.asarray(x, np.float32)
        return x.mean(axis=1) if x.ndim > 1 else x
    except Exception:
        return None


def sound_model():
    """The sound-recognition model's file (YAMNet, 521 kinds of sound; fetch_models.py), or None if it isn't here."""
    import paths
    p = paths.ROOT / "models" / "yamnet.tflite"
    return p if p.exists() and p.stat().st_size > 1_000_000 else None


_classifier = {"made": None, "lock": None}


def sounds(x, rate=RATE, top=6):
    """What kinds of sound these seconds hold, by the sound-recognition model: {kind: share 0..1} of the `top`
    kinds, each the model's average score over the seconds ("Music", "Speech", "Silence", "Television", "Typing",
    "Air conditioning"...). None without the model or MediaPipe. A judgement of the sound, with its score: a
    microphone in a room hears whatever is there."""
    model = sound_model()
    if model is None or x is None or len(x) < rate:
        return None
    import face_id
    if not face_id.mediapipe_usable()[0]:            # e.g. a processor MediaPipe's build would end the process on
        return None
    try:
        import threading

        import numpy as np
        from mediapipe.tasks import python as tasks
        from mediapipe.tasks.python import audio
        from mediapipe.tasks.python.components import containers
        if _classifier["lock"] is None:
            _classifier["lock"] = threading.Lock()
        with _classifier["lock"]:                    # one model, one listening at a time
            if _classifier["made"] is None:
                _classifier["made"] = audio.AudioClassifier.create_from_options(audio.AudioClassifierOptions(
                    base_options=tasks.BaseOptions(model_asset_buffer=model.read_bytes()), max_results=12,
                    running_mode=audio.RunningMode.AUDIO_CLIPS))
            # the model was taught on sound at ordinary loudness: a quiet microphone is brought up to it first
            x = np.asarray(x, np.float32)
            x = x - float(x.mean())
            peak = float(np.abs(x).max())
            clip = containers.AudioData.create_from_array(x / peak * 0.5 if peak > 1e-5 else x, rate)
            results = _classifier["made"].classify(clip)
        total, n = collections.Counter(), 0
        for r in results:
            n += 1
            for c in r.classifications[0].categories:
                total[c.category_name] += c.score
        if not n:
            return None
        return {k: round(v / n, 3) for k, v in total.most_common(top) if v / n >= 0.02}
    except Exception:
        return None


def ambient(x, rate=RATE):
    """The background of a sound as numbers, for what isn't a song: {level, floor, events, steady, flat, hum, low,
    high, speechlike} or None when it is silence.
      level   how loud overall (dBFS)             floor  the level it rarely goes under: the steady background
      events  dB between that floor and its loud moments (things happening over the background)
      steady  0..1, 1 = the loudness hardly moves (a fan, a hum), 0 = it comes and goes (talk, music, activity)
      flat    0..1, 1 = hiss-like noise with no tone, 0 = pure tones
      hum     dB by which mains hum (50 or 60 Hz and its multiples) stands out of its surroundings
      low, high  the shares of the energy under 150 Hz (rumble) and over 4 kHz (hiss, clatter)
      speechlike  0..1, how much of the loudness of the voice range comes and goes at the rate of syllables"""
    import numpy as np
    x = np.asarray(x, np.float32)
    if len(x) < rate * 2:
        return None
    x = x - float(x.mean())                          # an input's constant offset is no sound
    level = 20 * math.log10(float(np.sqrt(np.mean(x ** 2))) + 1e-12)
    if level < SILENT - 15:
        return None
    n, hop = 2048, 512
    frames = 1 + (len(x) - n) // hop
    idx = np.arange(n)[None, :] + hop * np.arange(frames)[:, None]
    power = np.abs(np.fft.rfft(x[idx] * np.hanning(n), axis=1)) ** 2
    freq = np.fft.rfftfreq(n, 1 / rate)
    total = power.sum(axis=1) + 1e-20
    db = 10 * np.log10(np.mean(x[idx] ** 2, axis=1) + 1e-24)       # each moment's level, in dBFS like `level`
    floor, loud = float(np.percentile(db, 10)), float(np.percentile(db, 95))
    mean = power.mean(axis=0) + 1e-20
    band = (freq > 100) & (freq < 8000)
    flat = float(np.exp(np.mean(np.log(mean[band]))) / np.mean(mean[band]))
    hum = 0.0
    for base in (50.0, 60.0):                        # the mains tone and its first multiples against their surroundings
        lines = []
        for k in (1, 2, 3):
            i = int(round(base * k * n / rate))
            near = np.r_[mean[max(1, i - 6):i - 1], mean[i + 2:i + 7]]
            lines.append(10 * math.log10(float(mean[i - 1:i + 2].max()) / (float(np.median(near)) + 1e-20)))
        hum = max(hum, float(np.median(lines)))
    voice = power[:, (freq > 300) & (freq < 3400)].sum(axis=1)
    env = voice / (voice.mean() + 1e-20) - 1
    mod = np.abs(np.fft.rfft(env * np.hanning(len(env)))) ** 2
    mfreq = np.fft.rfftfreq(len(env), hop / rate)
    speechlike = float(mod[(mfreq >= 2) & (mfreq <= 8)].sum() / (mod[mfreq > 0.3].sum() + 1e-20))
    r = lambda v, d=1: round(float(v), d)            # noqa: E731
    return {"level": r(level), "floor": r(floor), "events": r(loud - floor), "steady": r(max(0.0, 1 - float(np.std(db)) / 6), 2),
            "flat": r(flat, 3), "hum": r(max(0.0, hum)), "low": r(power[:, freq < 150].sum() / total.sum(), 3),
            "high": r(power[:, freq > 4000].sum() / total.sum(), 3), "speechlike": r(speechlike, 2)}


# The model's 521 kinds of sound, gathered into the few families a background is described by. A kind belongs to
# the first family one of whose words is in its name (lower case). Not measured against anything: a way to say the
# model's answer in plain words, shown with its score.
FAMILIES = (
    ("quiet", ("silence",)),
    ("animal sounds", ("animal", "dog", "bark", "cat", "meow", "bird", "chirp", "purr", "insect", "cricket")),   # before
    ("music", ("music", "singing",                                              # music: "bird song" is a bird "song", "guitar", "piano", "drum", "bass", "synth", "violin", "choir", "orchestra",
               "instrument", "trumpet", "saxophone", "flute", "keyboard (musical)", "organ", "harmonica", "rapping",
               "humming", "chant", "beatboxing", "strum", "percussion", "cymbal", "jingle", "soundtrack")),
    ("television or radio", ("television", "radio")),
    ("speech", ("speech", "conversation", "narration", "monologue", "babbling", "whispering", "shout", "yell",
                "laughter", "chatter", "crowd", "hubbub")),
    ("steady background noise", ("noise", "static", "hum", "air conditioning", "fan", "engine", "idling", "vehicle",
                                 "truck", "car", "wind", "water", "stream", "rain", "ocean", "waves", "rumble",
                                 "inside,", "outside,", "white", "pink", "drone", "buzz", "motor", "aircraft",
                                 "train", "traffic", "hiss", "vibration", "throbbing", "ventilation")),
    ("alert sounds", ("alarm", "telephone", "ringtone", "beep", "doorbell", "siren", "buzzer", "smoke detector",
                      "bell", "chime", "ding")),
    ("activity", ("typing", "keyboard", "click", "mouse", "door", "footsteps", "walk", "dishes", "cutlery", "tick",
                  "knock", "writing", "rustle", "chair", "drawer", "tap", "thump", "clatter", "scissors", "zipper",
                  "cupboard", "printer", "breathing", "cough", "sneeze", "chewing", "sniff", "clapping", "bang",
                  "squeak", "crumpling", "tearing", "paper", "sound effect")),
)


def families(sounds_):
    """The model's kinds of sound added up by family: {family: share}, most first."""
    out = collections.Counter()
    for kind, score in (sounds_ or {}).items():
        low = kind.lower()
        out[next((fam for fam, words in FAMILIES if any(w in low for w in words)), "other sounds")] += score
    return {k: round(min(1.0, v), 3) for k, v in out.most_common()}


def background(amb):
    """A steady background in plain words from its numbers: what kind of noise it is."""
    if not amb:
        return ""
    if amb["level"] >= -12 and amb["steady"] >= 0.8:  # near full strength and never changing: no room sounds so
        return "an input with nothing plugged in, probably"
    parts = []
    if amb["hum"] >= 10:
        parts.append("mains hum")
    if amb["low"] >= 0.5:
        parts.append("low rumble")
    elif amb["high"] >= 0.3:
        parts.append("hiss")
    elif amb["flat"] >= 0.3:
        parts.append("broadband, fan-like")
    return ", ".join(parts)


def what(sounds_, amb):
    """In a few words what an interface heard, and how sure (0..1): "quiet", "music", "speech", "television or
    radio", "steady background noise", "activity", "alert sounds", "animal sounds" or "other sounds". From the
    model's kinds of sound when it is here; without it only from how the background behaves (then never music or
    speech). An opinion with its score, not a fact about the room. Not measured against anything yet."""
    if amb is None or amb["level"] < SILENT:
        return "quiet", 1.0 if amb is None else 0.8
    fam = families(sounds_)
    if fam:
        first, score = next(iter(fam.items()))
        if fam.get("music", 0) >= 0.25 and fam["music"] >= fam.get("speech", 0):
            return "music", fam["music"]
        if fam.get("speech", 0) >= 0.25:
            return "speech", fam["speech"]
        # a background that hardly moves is a steady noise, whatever single thing the model likens it to
        if amb["steady"] >= 0.7 and first not in ("quiet", "music", "speech"):
            return "steady background noise", round(max(fam.get("steady background noise", 0), amb["steady"] * 0.6), 2)
        return first, score
    return ("steady background noise" if amb["steady"] > 0.7 else "sound that comes and goes"), 0.5


def listen_all(seconds=10.0, kinds=("output", "input")):
    """Every interface listened to at the same moment for `seconds`: [{name, kind, default, heard, sounds, ambient,
    what, sure}]. heard: measure()'s numbers as for a song (None when silent); sounds: the kinds of sound the
    model hears; ambient: the background's numbers; what and sure: those in a word ("music", "speech", "quiet"...)
    and how sure. So what plays is known by where it sounded: from this computer through one of its outputs, or
    in the room, heard by a microphone."""
    import threading
    found = [i for i in interfaces() if i["kind"] in kinds]
    got = {}

    def one(i):
        if platform.system() == "Windows":           # each thread says it uses Windows' sound objects
            try:
                import ctypes
                ctypes.windll.ole32.CoInitializeEx(None, 0)
            except Exception:
                pass
        got[i["id"], i["kind"]] = record(i, seconds)
    threads = [threading.Thread(target=one, args=(i,), daemon=True) for i in found]
    for t in threads:
        t.start()
    for t in threads:
        t.join(seconds + 15)
    out = []
    for i in found:
        x = got.get((i["id"], i["kind"]))
        amb = None if x is None else ambient(x)
        snd = sounds(x) if amb is not None else None
        kind, sure = what(snd, amb)
        out.append({"name": i["name"], "kind": i["kind"], "default": i["default"],
                    # a song's numbers (tempo, key) only where music is what sounds: a fan has no key
                    "heard": measure(x) if x is not None and (kind == "music" or (i["kind"] == "output" and snd is None)) else None,
                    "sounds": snd, "families": families(snd) or None, "ambient": amb, "what": kind, "sure": sure,
                    "background": background(amb) if kind == "steady background noise" else ""})
    return out


def measure(x, rate=RATE):
    """The sound as numbers: {level, dynamics, tempo, pulse, bright, bass, change, key, mode, mode_sure} or None
    when it is silence. level dBFS; dynamics dB between its quiet and loud moments; tempo beats a minute and pulse
    0..1 how clear that beat is; bright the centre of its spectrum in Hz; bass the share of its energy under
    150 Hz; change how much the spectrum moves from moment to moment; key and mode (major/minor) with mode_sure
    0..1. Measured values of the sound, nothing about the listener."""
    import numpy as np
    x = np.asarray(x, np.float32)
    if len(x) < rate * 2:
        return None
    x = x - float(x.mean())                          # an input's constant offset is no sound
    level = 20 * math.log10(float(np.sqrt(np.mean(x ** 2))) + 1e-12)
    if level < SILENT:
        return None
    n, hop = 2048, 512
    frames = 1 + (len(x) - n) // hop
    idx = np.arange(n)[None, :] + hop * np.arange(frames)[:, None]
    spec = np.abs(np.fft.rfft(x[idx] * np.hanning(n), axis=1))              # frames x bins
    freq = np.fft.rfftfreq(n, 1 / rate)
    power = spec ** 2
    total = power.sum(axis=1) + 1e-12
    rms = 10 * np.log10(total / n + 1e-12)
    heard = rms > rms.max() - 40                                            # leave the gaps out
    centroid = float(((power * freq).sum(axis=1) / total)[heard].mean())
    bass = float((power[:, freq < 150].sum(axis=1) / total)[heard].mean())
    dynamics = float(np.percentile(rms[heard], 95) - np.percentile(rms[heard], 10))
    logs = np.log1p(100 * spec)
    flux = np.maximum(logs[1:] - logs[:-1], 0).sum(axis=1)                  # how much is new in each frame
    change = float(flux.mean() / (logs.mean() * logs.shape[1] + 1e-9))
    # tempo: the lag at which the pattern of new sound repeats best, between 50 and 200 beats a minute
    env = flux - flux.mean()
    acf = np.correlate(env, env, "full")[len(env) - 1:]
    per_frame = hop / rate
    lo, hi = int(60 / 200 / per_frame), min(len(acf) - 1, int(60 / 50 / per_frame))
    tempo = pulse = None
    if hi > lo + 2 and acf[0] > 0:
        lags = np.arange(lo, hi)
        bpm = 60 / (lags * per_frame)
        weight = np.exp(-0.5 * (np.log2(bpm / 110) / 0.9) ** 2)             # people hear the beat near 110
        best = int(np.argmax(acf[lo:hi] * weight))
        tempo, pulse = float(bpm[best]), float(max(0.0, acf[lo + best] / acf[0]))
    # key and mode: how much of each of the 12 notes sounds, against the major and minor profiles
    use = (freq >= 55) & (freq <= 2000)
    pitch = np.round(12 * np.log2(freq[use] / 440.0) + 69).astype(int) % 12
    chroma = np.zeros(12)
    np.add.at(chroma, pitch, power[heard][:, use].sum(axis=0))
    key = mode = sure = None
    if chroma.sum() > 0:
        chroma = chroma / chroma.sum()
        fits = []
        for name, prof in (("major", MAJOR), ("minor", MINOR)):
            for k in range(12):
                fits.append((float(np.corrcoef(chroma, np.roll(prof, k))[0, 1]), NOTES[k], name))
        fits.sort(reverse=True)
        key, mode = fits[0][1], fits[0][2]
        other = next(f[0] for f in fits if f[2] != mode)
        sure = float(max(0.0, min(1.0, (fits[0][0] - other) * 4)))
    r = lambda v, d=1: None if v is None else round(v, d)       # noqa: E731
    return {"level": r(level), "dynamics": r(dynamics), "tempo": r(tempo, 0), "pulse": r(pulse, 2), "bright": r(centroid, 0),
            "bass": r(bass, 3), "change": r(change, 4), "key": key, "mode": mode, "mode_sure": r(sure, 2)}


def best(heard):
    """Of listen_all()'s interfaces, the one to take a song's numbers from: an output that sounds (the song
    exactly as played) before any microphone (the song through a room), the loudest of its kind. None if all
    were silent."""
    sounding = [h for h in heard if h.get("heard")]
    if not sounding:
        return None
    return max(sounding, key=lambda h: (h["kind"] == "output", h["heard"]["level"]))


def combine(measures):
    """One set of numbers for a song from the several times it was listened to: the median of each, the key and
    mode heard most."""
    measures = [m for m in measures if m]
    if not measures:
        return None
    out = {"heard": len(measures)}
    for k in ("level", "dynamics", "tempo", "pulse", "bright", "bass", "change", "mode_sure"):
        vals = [m[k] for m in measures if m.get(k) is not None]
        out[k] = round(statistics.median(vals), 4) if vals else None
    for k in ("key", "mode"):
        vals = [m[k] for m in measures if m.get(k)]
        out[k] = collections.Counter(vals).most_common(1)[0][0] if vals else None
    return out


# --- a person's usual music -------------------------------------------------------------------------------------

def _year(text):
    return int(text[:4]) if isinstance(text, str) and text[:4].isdigit() and 1900 <= int(text[:4]) <= 2100 else None


def _top(counter, n, total):
    return [{"name": k, "songs": v, "share": round(v / total, 4)} for k, v in counter.most_common(n)]


def picture(songs, genres=None, now=None):
    """A person's usual music from their liked songs (spotify.liked) and their artists' genres (spotify.genres):
    {songs, artists, top_artists, genres, decades, year, added, ...}: counts, shares and short names only."""
    genres = genres or {}
    songs = [s for s in songs if isinstance(s, dict) and s.get("name")]
    n = len(songs)
    if not n:
        return {"songs": 0}
    artists, genre_count, decades, added = collections.Counter(), collections.Counter(), collections.Counter(), collections.Counter()
    years, with_genre = [], 0
    for s in songs:
        first = (s.get("artists") or [{}])[0]
        if first.get("name"):
            artists[first["name"]] += 1
        seen = set()
        for a in s.get("artists") or []:
            seen.update(genres.get(a.get("id"), []))
        with_genre += bool(seen)
        genre_count.update(seen)
        y = _year(s.get("released"))
        if y:
            years.append(y)
            decades[f"{y // 10 * 10}s"] += 1
        a = _year(s.get("added"))
        if a:
            added[str(a)] += 1
    pop = [s["popularity"] for s in songs if isinstance(s.get("popularity"), int)]
    mins = [s["ms"] / 60000 for s in songs if s.get("ms")]
    shares = [v / n for v in artists.values()]
    out = {
        "songs": n, "artists": len(artists), "made": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now)),
        "top_artists": _top(artists, 25, n),
        # every artist with more than one liked song (place() ranks a song's artist among them)
        "by_artist": dict(sorted(((k, v) for k, v in artists.items() if v > 1), key=lambda kv: -kv[1])[:3000]),
        "top_10_artists_share": round(sum(v for _, v in artists.most_common(10)) / n, 4),
        # how spread out the liking is: 1 = every song by another artist, near 0 = nearly all by one
        "spread": round(-sum(p * math.log(p) for p in shares) / math.log(len(shares)), 4) if len(shares) > 1 else 0.0,
        "genres": _top(genre_count, 25, n), "songs_with_genres": round(with_genre / n, 4),
        "decades": [{"name": d, "songs": c, "share": round(c / n, 4)} for d, c in sorted(decades.items())],
        "year": {"median": int(statistics.median(years)), "oldest": min(years), "newest": max(years),
                 "middle_half": [int(statistics.quantiles(years, n=4)[0]), int(statistics.quantiles(years, n=4)[2])]
                 if len(years) >= 4 else None} if years else None,
        "added": [{"year": y, "songs": c} for y, c in sorted(added.items())],
        "explicit_share": round(sum(bool(s.get("explicit")) for s in songs) / n, 4),
        "minutes": {"median": round(statistics.median(mins), 2), "total_hours": round(sum(mins) / 60, 1)} if mins else None,
        # Spotify's 0..100 for how much a song is played by everyone now: low = little known
        "popularity": {"median": int(statistics.median(pop)), "little_known_share": round(sum(p < 25 for p in pop) / len(pop), 4),
                       "widely_played_share": round(sum(p >= 60 for p in pop) / len(pop), 4)} if pop else None,
    }
    return out


def place(artist, song, pic, artist_genres=None):
    """One song against a person's picture: {artist_rank, artist_songs, genre_overlap, usual}. usual is
    "one of their artists", "their kind of music" or "not their usual", from the artist's place in the liked
    songs and, when the artist's genres are known, how much of the liked collection shares them."""
    liked = pic.get("by_artist") or {a["name"]: a["songs"] for a in pic.get("top_artists", [])}
    names = [a.lower() for a in liked]                           # most liked first
    rank = names.index(artist.lower()) + 1 if artist.lower() in names else None
    count = next((n for a, n in liked.items() if a.lower() == artist.lower()), 0)
    share = {g["name"]: g["share"] for g in pic.get("genres", [])}
    overlap = round(max((share.get(g, 0) for g in artist_genres or []), default=0), 4) if artist_genres else None
    usual = ("one of their artists" if rank else "their kind of music" if overlap and overlap >= 0.1
             else "not their usual" if overlap is not None or pic.get("songs") else None)
    return {"artist": artist[:200], "song": song[:200], "artist_rank": rank, "artist_songs": count,
            "genre_overlap": overlap, "usual": usual}
