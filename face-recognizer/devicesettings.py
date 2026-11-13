"""This device's page settings and what the tracker learned about each person, kept by the servers for the pages
(page_store.js), so they outlast the browser's storage and reach the shared database (dbsync_home: device_settings,
one set per device; person_signals, shared by every device).

  HOME/settings.json        {app: {key: value}}   app: "track" (ft-... keys) or "label" (fl-... keys)
  HOME/person_signals.json  {kind: {name: data}}  kind: "usual" (Expression.UsualLooks) or "baseline" (Signals.Baselines)

Values are the pages' own localStorage strings. Nothing here is a measured threshold or a default: the pages keep
those. Without a shared database these files are this device's own copy, as browser storage was before."""
import json
import re
import threading

import paths

SETTINGS = "settings.json"
SIGNALS = "person_signals.json"
APPS = {"track": "ft-", "label": "fl-"}
KINDS = {"usual": "ft-usual", "baseline": "ft-baselines"}       # kind -> the page's localStorage key
NOT_SETTINGS = set(KINDS.values()) | {"ft-forgot-done"}         # person data or page bookkeeping, never a setting
MAX_VALUE = 20_000
MAX_SIGNALS = 5_000_000
_lock = threading.Lock()


def _read(name):
    v = paths.read_json(paths.HOME / name, {})     # OSError if it's there and can't be read: never taken as empty
    return v if isinstance(v, dict) else {}


def _write(name, obj):
    paths.atomic_write(paths.HOME / name, json.dumps(obj, ensure_ascii=False, indent=0))


def settings(app=None):
    with _lock:
        s = _read(SETTINGS)
    return s if app is None else dict(s.get(app) or {})


def update(app, changes):
    """Set (or with None, remove) some of app's settings. Checked: keys of that page, short string values."""
    if app not in APPS or not isinstance(changes, dict):
        raise ValueError("bad settings")
    for k, v in changes.items():
        if not isinstance(k, str) or not k.startswith(APPS[app]) or len(k) > 80 or k in NOT_SETTINGS \
                or not re.fullmatch(r"[\w.-]+", k):
            raise ValueError("bad setting name")
        if v is not None and (not isinstance(v, str) or len(v) > MAX_VALUE):
            raise ValueError("bad setting value")
    with _lock:
        s = _read(SETTINGS)
        mine = s.setdefault(app, {})
        for k, v in changes.items():
            if v is None:
                mine.pop(k, None)
            else:
                mine[k] = v
        _write(SETTINGS, s)
        return dict(mine)


def replace_settings(all_settings):
    """The whole {app: {key: value}} (the sync, when another device's settings were copied to this one)."""
    with _lock:
        _write(SETTINGS, {a: dict(v) for a, v in all_settings.items() if a in APPS and isinstance(v, dict)})


def signals():
    with _lock:
        s = _read(SIGNALS)
    return {k: dict(s.get(k) or {}) for k in KINDS}


def _forgotten():
    try:
        import dbsync_home
        return dbsync_home.forgotten_names()
    except ImportError:
        return set()


def merge_signals(kind, changed, gone=()):
    """What the tracker page learned since it last said: changed {name: data} is set, the names in gone are removed.
    Nobody else is touched. (The page used to send its whole map, and whoever it didn't know of was dropped: people
    another device had learned since the page loaded, here and through the sync on every device.) A forgotten name
    is refused until it is given to someone again, so a page still running can't bring a forgotten person back."""
    if kind not in KINDS or not isinstance(changed, dict) or not isinstance(gone, (list, tuple)):
        raise ValueError("bad signals")
    if len(json.dumps(changed)) > MAX_SIGNALS:
        raise ValueError("too much")
    forgotten = _forgotten()
    with _lock:
        s = _read(SIGNALS)
        m = s[kind] = dict(s.get(kind) or {})
        for n, d in changed.items():
            n = str(n)[:100]
            if isinstance(d, dict) and n and not n.startswith("#") and n not in forgotten:
                m[n] = d
        for n in gone:
            m.pop(str(n), None)
        _write(SIGNALS, s)


def signals_state(have=None):
    """For the tracker page's check every half minute: {v} and, unless the page already has version `have`,
    {signals, forgotten} (what other devices learned or forgot arrives in the file through the sync)."""
    def stamp(p):
        try:
            st = p.stat()
            return f"{st.st_mtime_ns}.{st.st_size}"
        except OSError:
            return "-"
    v = stamp(paths.HOME / SIGNALS) + "/" + stamp(paths.HOME / "forgotten.json")
    if have == v:
        return {"v": v, "same": True}
    return {"v": v, "signals": {KINDS[k]: p for k, p in signals().items()}, "forgotten": sorted(_forgotten())}


def set_signals(kind, people):
    """The whole map of one kind ({name: data}); a person missing from it is dropped. Not for the page (see
    merge_signals): it only knows the people it has seen."""
    if kind not in KINDS or not isinstance(people, dict):
        raise ValueError("bad signals")
    if len(json.dumps(people)) > MAX_SIGNALS:
        raise ValueError("too much")
    with _lock:
        s = _read(SIGNALS)
        s[kind] = {str(n)[:100]: d for n, d in people.items() if isinstance(d, dict) and n and not n.startswith("#")}
        _write(SIGNALS, s)


def put_signal(kind, name, data):
    """One person's row (the sync); data None removes it."""
    with _lock:
        s = _read(SIGNALS)
        m = s.setdefault(kind, {})
        if data is None:
            m.pop(name, None)
        else:
            m[name] = data
        _write(SIGNALS, s)


def forget(name):
    """Drop name from every kind (persondata.forget). Returns how many rows went."""
    with _lock:
        s = _read(SIGNALS)
        n = sum(1 for k in KINDS if name in (s.get(k) or {}))
        for k in KINDS:
            (s.get(k) or {}).pop(name, None)
        if n:
            _write(SIGNALS, s)
        return n


def of(name):
    """{kind: data} held about name (persondata.export)."""
    s = signals()
    return {k: s[k][name] for k in KINDS if name in s[k]}


def page_state(app):
    """What a page restores at load: its settings, and for the tracker the person signals and forgotten names."""
    out = {"settings": settings(app)}
    if app == "track":
        out["signals"] = {KINDS[k]: v for k, v in signals().items()}
        out["forgotten"] = sorted(_forgotten())
    return out
