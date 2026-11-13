"""The private folder's part of the shared database's sync (dbsync.py runs it next to the labeling data's part):
photo descriptions, Collect faces snapshots, the alert inbox, the presence log, shared fitted documents, keys
(encrypted, secretbox.py) and people forgotten on another device. Plus three things only ever sent: the app log, the
posed expression tests and the Web search use counts (count_use, which also holds each service's cap across devices).

Each table is read from and written to the same files the programs use, through their own modules and locks, so the
programs don't change: they keep reading their files. A device's own presence visits and snapshots keep their device;
those from other devices are written into this device's files too, so its pages show everyone's."""
import hashlib
import json
import re
import threading
import time
from datetime import datetime, timezone

import facedb
import paths

SHARED_DOCS = ("expr_calibration", "va_fit")      # data/<name>.json, fitted from everyone's reports
DEVICE_DOCS = ("model_eval", "hw_bench", "webcam_eval")
LOG_KEEP_DAYS = 90
MOVE_WINDOW = 600            # seconds: seen on another camera this recently counts as moving between cameras
KEEP_SIGHTINGS = 2 * MOVE_WINDOW   # seconds: older sightings are deleted (only the move alert uses them)
FORGOTTEN = "forgotten.json"                      # HOME: names forgotten here or applied from other devices
_forgot_lock = threading.Lock()


def _h(obj):
    return hashlib.sha1(json.dumps(obj, sort_keys=True, default=str).encode()).hexdigest()


def forgotten_names(strict=False):
    """Names forgotten here or on another device, and not given to anyone since (unforget). strict (the sync): a
    file that is there but can't be read raises OSError instead of counting as "no names", which the sync would
    send on as every forget withdrawn."""
    try:
        return {n for n in json.loads((paths.HOME / FORGOTTEN).read_text("utf-8")) if isinstance(n, str)}
    except FileNotFoundError:
        return set()
    except (OSError, ValueError, TypeError) as e:
        if strict:
            raise OSError(f"can't read {FORGOTTEN}: {e}") from e
        return set()


def note_forgotten(name):
    """persondata.forget calls this: the forget goes to the other devices with the next sync."""
    with _forgot_lock:
        names = forgotten_names() | {name}
        paths.atomic_write(paths.HOME / FORGOTTEN, json.dumps(sorted(names)))


def unforget(name):
    """The name was given to someone again (LabelDB.act, rename), or another device said so: the earlier forget no
    longer applies. Without this the name stayed an instruction for good, and a device joining later (or back
    after a long time away) forgot whoever carried the name by then, on every device. True if it was forgotten."""
    with _forgot_lock:
        names = forgotten_names()
        if name not in names:
            return False
        paths.atomic_write(paths.HOME / FORGOTTEN, json.dumps(sorted(names - {name})))
        return True


class LimitReached(ValueError):
    pass


def count_use(service, month, cap):
    """Count one Web search in the shared database before it's sent; LimitReached past the cap (every device's
    searches count). Returns the number used this month. Raises facedb.NotConfigured / psycopg errors when the
    database can't be used (websearch then counts on this device alone)."""
    with facedb.connect() as conn:
        row = conn.execute("""INSERT INTO usage AS u (service, month, used) VALUES (%s, %s, 1)
                              ON CONFLICT (service, month) DO UPDATE SET used = u.used + 1 WHERE u.used < %s
                              RETURNING used""", (service, month, cap)).fetchone()
    if row is None:
        raise LimitReached(f"{service} limit reached: {cap} searches this month (keeps the bill under $10)")
    return row[0]


class HomePart:
    """forget(name): what to run when another device forgot someone (label_tool gives persondata.forget)."""
    TABLES = ("descriptions", "webcam_snapshots", "alerts", "presence", "documents", "secrets", "forgotten",
              "device_settings", "person_signals")
    KEYS = {"descriptions": "key", "webcam_snapshots": "name", "alerts": "id", "presence": "id", "documents": "id",
            "secrets": "name", "forgotten": "name", "device_settings": "device,app,key", "person_signals": "name,kind"}

    def __init__(self, forget=None, peer_provider=None):
        self.forget = forget
        self.peer_provider = peer_provider  # (hash, frame) -> JPEG: this device's photos for the others (peer.py)
        self.peer_service = None
        self.seen_before = set()            # names this device's camera saw at the last step (camera moves)
        self.key_current = False          # this device has the current passphrase (secretbox.catch_up): keys sync

    def keycol(self, tbl):
        return self.KEYS[tbl]

    # ---- local rows
    def digests(self):
        import secretbox
        out = {t: {} for t in self.TABLES}
        out["descriptions"] = {k: _h(v) for k, v in self._descriptions().items()}
        out["webcam_snapshots"] = self._snap_digests()
        out["alerts"] = {a["id"]: _h(a) for a in self._alerts()}
        out["presence"] = {k: _h(v) for k, v in self._presence().items()}
        out["documents"] = {k: _h(v) for k, v in self._documents().items()}
        key = secretbox.local_key()
        out["secrets"] = None if key is None or not self.key_current else \
            {n: secretbox.digest(key, n, p) for n, p in self._secrets().items()}
        out["forgotten"] = {n: _h(n) for n in forgotten_names(strict=True)}
        out["device_settings"] = {k: _h(v) for k, v in self._settings().items()}
        out["person_signals"] = {k: _h(v) for k, v in self._signals().items()}
        return out

    def _snap_digests(self):
        out = {}
        for jp in self._snapshots():
            try:
                st, js = jp.stat(), jp.with_suffix(".json")
                sj = js.stat() if js.exists() else None
                out[jp.stem] = _h([st.st_size, st.st_mtime_ns, sj and sj.st_size, sj and sj.st_mtime_ns])
            except OSError:
                continue
        return out

    def rows(self, tbl, keys):
        keys = set(keys)
        if tbl == "descriptions":
            return {k: v for k, v in self._descriptions().items() if k in keys}
        if tbl == "webcam_snapshots":
            out = {}
            for jp in self._snapshots():
                if jp.stem in keys:
                    try:
                        v = json.loads(jp.with_suffix(".json").read_text("utf-8"))
                    except (OSError, ValueError):
                        v = None
                    try:
                        out[jp.stem] = {"jpeg": jp.read_bytes(), "verdict": v}
                    except OSError:
                        continue
            return out
        if tbl == "alerts":
            return {a["id"]: a for a in self._alerts() if a["id"] in keys}
        if tbl == "presence":
            return {k: v for k, v in self._presence().items() if k in keys}
        if tbl == "documents":
            return {k: v for k, v in self._documents().items() if k in keys}
        if tbl == "secrets":
            return {k: v for k, v in self._secrets().items() if k in keys}
        if tbl == "device_settings":
            return {k: v for k, v in self._settings().items() if k in keys}
        if tbl == "person_signals":
            return {k: v for k, v in self._signals().items() if k in keys}
        return {n: {} for n in forgotten_names() if n in keys}

    @staticmethod
    def _settings():
        """{device|app|key: {app, key, value}}: the pages' settings, the photo describer's (but its key: a secret)
        and the LM Studio share folder, as this device has them."""
        import describe
        import devicesettings
        import lmstudio
        me = facedb.device()["id"]
        out = {}
        for app, kv in devicesettings.settings().items():
            for k, v in (kv or {}).items():
                out[f"{me}|{app}|{k}"] = {"app": app, "key": k, "value": v}
        c = describe.config()
        for k in ("use", "url", "model", "spread"):
            out[f"{me}|describe|{k}"] = {"app": "describe", "key": k, "value": c.get(k)}
        share = lmstudio.share()
        out[f"{me}|lmstudio|share"] = {"app": "lmstudio", "key": "share", "value": str(share) if share else ""}
        import hardware
        out[f"{me}|lmstudio|distributed"] = {"app": "lmstudio", "key": "distributed",
                                             "value": "1" if hardware.distributed() else "0"}
        return out

    @staticmethod
    def _signals():
        import devicesettings
        return {f"{name}|{kind}": {"name": name, "kind": kind, "data": d}
                for kind, m in devicesettings.signals().items() for name, d in m.items()}

    # ---- readers of this device's files
    @staticmethod
    def _descriptions():
        import describe
        with describe._lock:
            return {k: dict(v) for k, v in describe._load().items()}

    @staticmethod
    def _snapshots():
        import face_id as F
        return sorted(F.WEBCAM.glob("webcam-*.jpg")) if F.WEBCAM.is_dir() else []

    @staticmethod
    def _alerts():
        import alerts
        with alerts._lock:
            return alerts._load()

    @staticmethod
    def _presence():
        """{device:start:name: {name, start, end, device}} from every presence file."""
        import face_id as F
        me = facedb.device()["id"]
        out = {}
        if F.PRESENCE.is_dir():
            for f in sorted(F.PRESENCE.glob("*.jsonl")):
                for line in f.read_text("utf-8").splitlines():
                    try:
                        v = json.loads(line)
                        dev = v.get("device") or me
                        out[f"{dev}:{int(v['start'])}:{v['name']}"] = {"name": v["name"], "start": int(v["start"]),
                                                                        "end": int(v["end"]), "device": dev}
                    except (ValueError, KeyError, TypeError):
                        continue
        return out

    @staticmethod
    def _documents():
        import face_id as F
        me = facedb.device()["id"]
        out = {}
        for name in SHARED_DOCS + DEVICE_DOCS:
            p = F.DATA / f"{name}.json"
            try:
                body = json.loads(p.read_text("utf-8"))
            except (OSError, ValueError):
                continue
            out[name if name in SHARED_DOCS else f"{me}:{name}"] = {"name": name, "body": body,
                                                                   "device": None if name in SHARED_DOCS else me}
        return out

    @staticmethod
    def _secrets():
        """{name: plain text} of the keys saved on this device (files only, not environment variables)."""
        import describe
        import websearch
        out = {}
        k = websearch.saved_key()
        if k:
            out["google_vision"] = k
        a = websearch.aws()
        if a:
            out["aws_rekognition"] = json.dumps({k: a.get(k) for k in ("access_key_id", "secret_access_key", "region")},
                                                sort_keys=True)
        c = describe.config()
        if c.get("key"):
            out[f"describe_key:{c['url']}"] = c["key"]
        return out

    # ---- to the server
    def push(self, conn, tbl, rows, deletes):
        import secretbox
        me = facedb.device()["id"]
        cur = conn.cursor()
        items = list(rows.items())
        with conn.transaction():
            if tbl == "descriptions":
                cur.executemany("""INSERT INTO descriptions (key, data) VALUES (%s, %s)
                                   ON CONFLICT (key) DO UPDATE SET data = excluded.data""",
                                [(k, json.dumps(_utc(v))) for k, v in items])
            elif tbl == "webcam_snapshots":
                cur.executemany("""INSERT INTO webcam_snapshots (name, device, taken, jpeg, verdict)
                                   VALUES (%s, %s, %s, %s, %s)
                                   ON CONFLICT (name) DO UPDATE SET jpeg = excluded.jpeg, verdict = excluded.verdict,
                                   taken = excluded.taken, device = coalesce(webcam_snapshots.device, excluded.device)""",
                                [(k, me, paths.parse_time((v["verdict"] or {}).get("when")), v["jpeg"],
                                  json.dumps(_utc(v["verdict"])) if v["verdict"] is not None else None)
                                 for k, v in items])
            elif tbl == "alerts":
                cur.executemany("""INSERT INTO alerts (id, device, data) VALUES (%s, %s, %s)
                                   ON CONFLICT (id) DO UPDATE SET data = excluded.data""",
                                [(k, me, json.dumps(v)) for k, v in items])
            elif tbl == "presence":
                cur.executemany("""INSERT INTO presence (id, device, name, start_ms, end_ms) VALUES (%s, %s, %s, %s, %s)
                                   ON CONFLICT (id) DO UPDATE SET end_ms = excluded.end_ms""",
                                [(k, v["device"], v["name"], v["start"], v["end"]) for k, v in items])
            elif tbl == "documents":
                cur.executemany("""INSERT INTO documents (id, device, name, body) VALUES (%s, %s, %s, %s)
                                   ON CONFLICT (id) DO UPDATE SET body = excluded.body""",
                                [(k, v["device"], v["name"], json.dumps(v["body"])) for k, v in items])
            elif tbl == "secrets":
                cur_v = secretbox.current(conn)
                if cur_v is None or secretbox.local_version() != cur_v["id"]:
                    self.key_current = False          # changed elsewhere meanwhile: take the new one first
                    return False                      # (nothing sent; the sync keeps these as not sent)
                key = secretbox.local_key()
                enc = [(k, *secretbox.encrypt(key, k, v)) for k, v in items]
                cur.executemany("""INSERT INTO secrets (name, nonce, ciphertext) VALUES (%s, %s, %s)
                                   ON CONFLICT (name) DO UPDATE SET nonce = excluded.nonce,
                                   ciphertext = excluded.ciphertext""", enc)
                # a describe key leaves this device when its server address changes: that's not a removal
                deletes = [k for k in deletes if not k.startswith("describe_key:")]
            elif tbl == "device_settings":
                cur.executemany("""INSERT INTO device_settings (device, app, key, value) VALUES (%s, %s, %s, %s)
                                   ON CONFLICT (device, app, key) DO UPDATE SET value = excluded.value""",
                                [(me, v["app"], v["key"], json.dumps(v["value"])) for _k, v in items])
                for k in deletes:
                    _dev, app, key = k.split("|", 2)
                    cur.execute("DELETE FROM device_settings WHERE device = %s AND app = %s AND key = %s",
                                (me, app, key))
                deletes = []
            elif tbl == "person_signals":
                cur.executemany("""INSERT INTO person_signals (name, kind, data) VALUES (%s, %s, %s)
                                   ON CONFLICT (name, kind) DO UPDATE SET data = excluded.data""",
                                [(v["name"], v["kind"], json.dumps(v["data"])) for _k, v in items])
                for k in deletes:
                    name, kind = k.rsplit("|", 1)
                    cur.execute("DELETE FROM person_signals WHERE name = %s AND kind = %s", (name, kind))
                deletes = []
            elif tbl == "forgotten":
                cur.executemany("INSERT INTO forgotten (name) VALUES (%s) ON CONFLICT (name) DO NOTHING",
                                [(k,) for k, _v in items])
                for name, _v in items:              # what the server holds about them goes at once
                    _forget_on_server(cur, name)
                if deletes:                         # named again here (unforget): no longer an instruction
                    cur.execute("DELETE FROM forgotten WHERE name = ANY(%s)", (list(deletes),))
                deletes = []
            if deletes and tbl in ("descriptions", "webcam_snapshots", "alerts", "presence", "secrets"):
                cur.execute(f"DELETE FROM {tbl} WHERE {self.KEYS[tbl]} = ANY(%s)", (list(deletes),))

    def fetch(self, conn, tbl, keys):
        out = {k: None for k in keys}
        keys = list(keys)
        me = facedb.device()["id"]
        for b in range(0, len(keys), 500):
            part = keys[b:b + 500]
            if tbl == "descriptions":
                for k, d in conn.execute("SELECT key, data FROM descriptions WHERE key = ANY(%s)", (part,)):
                    out[k] = d
            elif tbl == "webcam_snapshots":
                for k, jpeg, verdict in conn.execute(
                        "SELECT name, jpeg, verdict FROM webcam_snapshots WHERE name = ANY(%s)", (part,)):
                    out[k] = {"jpeg": bytes(jpeg), "verdict": verdict}
            elif tbl == "alerts":
                for k, d in conn.execute("SELECT id, data FROM alerts WHERE id = ANY(%s)", (part,)):
                    out[k] = d
            elif tbl == "presence":
                for k, dev, name, s, e in conn.execute(
                        "SELECT id, device::text, name, start_ms, end_ms FROM presence WHERE id = ANY(%s)", (part,)):
                    out[k] = {"name": name, "start": s, "end": e, "device": dev}
            elif tbl == "documents":
                for k, dev, name, body in conn.execute(
                        "SELECT id, device::text, name, body FROM documents WHERE id = ANY(%s)", (part,)):
                    if dev is None or dev == me:          # other devices' measurements stay theirs
                        out[k] = {"name": name, "body": body, "device": dev}
                    else:
                        out.pop(k, None)
            elif tbl == "secrets":
                for k, nonce, ct in conn.execute(
                        "SELECT name, nonce, ciphertext FROM secrets WHERE name = ANY(%s)", (part,)):
                    out[k] = {"nonce": bytes(nonce), "ciphertext": bytes(ct)}
            elif tbl == "device_settings":
                for k in part:
                    dev, app, key = k.split("|", 2)
                    if dev != me:                      # other devices' settings stay theirs
                        out.pop(k, None)
                        continue
                    row = conn.execute("SELECT value FROM device_settings WHERE device = %s AND app = %s AND key = %s",
                                       (me, app, key)).fetchone()
                    out[k] = {"app": app, "key": key, "value": row[0]} if row else None
            elif tbl == "person_signals":
                for name, kind, data in conn.execute(
                        "SELECT name, kind, data FROM person_signals WHERE name || '|' || kind = ANY(%s)", (part,)):
                    out[f"{name}|{kind}"] = {"name": name, "kind": kind, "data": data}
            else:
                for (k,) in conn.execute("SELECT name FROM forgotten WHERE name = ANY(%s)", (part,)):
                    out[k] = {}
        return out

    def all_keys(self, conn, tbl):
        if tbl == "device_settings":
            me = facedb.device()["id"]
            return [f"{me}|{a}|{k}" for a, k in conn.execute(
                "SELECT app, key FROM device_settings WHERE device = %s", (me,))]
        if tbl == "person_signals":
            return [f"{n}|{k}" for n, k in conn.execute("SELECT name, kind FROM person_signals")]
        if tbl == "documents":
            me = facedb.device()["id"]
            return [r[0] for r in conn.execute("SELECT id FROM documents WHERE device IS NULL OR device = %s", (me,))]
        return [r[0] for r in conn.execute(f"SELECT {self.KEYS[tbl]} FROM {tbl}")]

    # ---- from the server
    def apply(self, tbl, rows, log=print):
        import secretbox
        if tbl == "descriptions":
            import describe
            with describe._lock:
                s = describe._load()
                for k, v in rows.items():
                    if v is None:
                        s.pop(k, None)
                    else:
                        s[k] = v
                describe._write(s)
                return {k: _h(s[k]) if k in s else None for k in rows}
        if tbl == "webcam_snapshots":
            import face_id as F
            F.WEBCAM.mkdir(parents=True, exist_ok=True)
            out = {}
            for k, v in rows.items():
                jp = F.WEBCAM / f"{k}.jpg"
                if not re.fullmatch(r"webcam-[\w-]+", k):
                    continue
                if v is None:
                    jp.unlink(missing_ok=True)
                    jp.with_suffix(".json").unlink(missing_ok=True)
                    out[k] = None
                    continue
                if v.get("verdict") is not None:     # the verdict first: the labeling tool reads it with the picture
                    paths.atomic_write(jp.with_suffix(".json"), json.dumps(v["verdict"]))
                paths.atomic_write(jp, v["jpeg"])
                out[k] = None
            d = self._snap_digests()
            return {k: d.get(k) for k in out}
        if tbl == "alerts":
            import alerts
            with alerts._lock:
                have = {a["id"]: a for a in alerts._load()}
                for k, v in rows.items():
                    if v is None:
                        have.pop(k, None)
                    else:
                        have[k] = v
                alerts._save(sorted(have.values(), key=lambda a: a.get("t", 0)))
            return {k: _h(have[k]) if k in have else None for k in rows}
        if tbl == "presence":
            import face_id as F
            import time
            F.PRESENCE.mkdir(parents=True, exist_ok=True)
            me = facedb.device()["id"]
            have = self._presence()
            gone = {k for k, v in rows.items() if v is None and k in have}
            for k, v in rows.items():
                if v is None or k in have:
                    continue
                line = {"name": v["name"], "start": v["start"], "end": v["end"]}
                if v["device"] != me:
                    line["device"] = v["device"]
                day = time.strftime("%Y-%m-%d", time.localtime(v["start"] / 1000))   # the tracker's day files
                with open(F.PRESENCE / f"{day}.jsonl", "a", encoding="utf-8") as f:
                    f.write(json.dumps(line) + "\n")
            if gone:
                for f in F.PRESENCE.glob("*.jsonl"):
                    keep = []
                    for ln in f.read_text("utf-8").splitlines():
                        try:
                            v = json.loads(ln)
                            if f"{v.get('device') or me}:{int(v['start'])}:{v['name']}" in gone:
                                continue
                        except (ValueError, KeyError, TypeError):
                            pass
                        keep.append(ln)
                    paths.atomic_write(f, "".join(x + "\n" for x in keep))
            now = self._presence()
            return {k: _h(now[k]) if k in now else None for k in rows}
        if tbl == "documents":
            import face_id as F
            for v in rows.values():
                if v is not None and v["name"] in SHARED_DOCS:
                    paths.atomic_write(F.DATA / f"{v['name']}.json", json.dumps(v["body"], indent=1))
            now = self._documents()
            return {k: _h(now[k]) if k in now else None for k in rows}
        if tbl == "secrets":
            key = secretbox.local_key() if self.key_current else None   # an old passphrase reads nothing
            if key is None:
                return {}
            import describe
            import websearch
            for k, v in rows.items():
                try:
                    plain = None if v is None else secretbox.decrypt(key, k, v["nonce"], v["ciphertext"]).decode()
                except secretbox.WrongPassphrase as e:
                    log(f"Shared database: key {k}: {e}")
                    continue
                if k == "google_vision":
                    websearch.save_key(plain or "")
                elif k == "aws_rekognition":
                    a = json.loads(plain) if plain else {}
                    websearch.save_aws(a.get("access_key_id") or "", a.get("secret_access_key"), a.get("region"))
                elif k.startswith("describe_key:") and describe.config()["url"] == k.partition(":")[2]:
                    describe.set_config(key=plain or "")
            now = self._secrets()
            return {k: secretbox.digest(key, k, now[k]) if k in now else None for k in rows}
        if tbl == "device_settings":
            import describe
            import devicesettings
            import lmstudio
            pages = {}
            for k, v in rows.items():
                _dev, app, key = k.split("|", 2)
                value = None if v is None else v["value"]
                if app in devicesettings.APPS:
                    pages.setdefault(app, {})[key] = None if value is None else str(value)
                elif app == "describe" and value is not None and key in ("use", "url", "model", "spread"):
                    try:
                        describe.set_config(**{key: value})
                    except ValueError as e:
                        log(f"Shared database: describe setting {key}: {e}")
                elif app == "lmstudio" and key == "distributed" and value is not None:
                    lmstudio.set_distributed(str(value) == "1")
                elif app == "lmstudio" and key == "share" and value is not None:
                    try:
                        lmstudio.set_share(value)
                    except ValueError as e:
                        log(f"Shared database: {e}")
            for app, changes in pages.items():
                try:
                    devicesettings.update(app, changes)
                except ValueError as e:
                    log(f"Shared database: settings: {e}")
            now = self._settings()
            return {k: _h(now[k]) if k in now else None for k in rows}
        if tbl == "person_signals":
            import devicesettings
            for k, v in rows.items():
                name, kind = k.rsplit("|", 1)
                if kind in devicesettings.KINDS:
                    devicesettings.put_signal(kind, name, None if v is None else v["data"])
            now = self._signals()
            return {k: _h(now[k]) if k in now else None for k in rows}
        # forgotten on another device: forget here too, once; named again there (the row is gone): no longer forgotten
        done = forgotten_names()
        for name, v in rows.items():
            if v is None:
                unforget(name)
            elif name not in done and self.forget:
                try:
                    self.forget(name)
                    log(f"Shared database: forgot {name}, as another device did.")
                except Exception as e:                # never stop the sync over one person
                    log(f"Shared database: couldn't forget {name} here: {e}")
                note_forgotten(name)
        return {n: _h(n) for n in rows if n in forgotten_names()}

    # ---- only sent: the app log, posed expression tests and the expression log, new lines since the last time
    def every_step(self, conn, state):
        """Each sync step (a few small queries): the passphrase for keys. Take the current one if it was set or
        changed elsewhere, give it to devices that lack it, and show this device's fingerprints. Taking a new one
        reads every key again under it."""
        import secretbox
        self.key_current, took = secretbox.catch_up(conn)
        login = facedb.saved_login() or {}
        inst = conn.execute("SELECT id FROM instance").fetchone()[0]
        secretbox.register(conn, login.get("password"), inst)
        self._peer(conn)
        self._cameras(conn)
        if took:
            keys = self.all_keys(conn, "secrets")
            state.set_digests("secrets", self.apply("secrets", self.fetch(conn, "secrets", keys)))

    def _cameras(self, conn):
        """This device's camera: whether it streams, who it sees (sightings), and a "moved" alert when someone it
        sees now was on another device's camera within MOVE_WINDOW (only this device raises it: the one they
        arrived at). The alert syncs to every device like the others.

        Sightings are opt-in like the presence log: only while "They move between cameras" is on, and only people
        on this device's alert list (the tracker's "alertWho"); nobody else's name and times are written. Rows older
        than KEEP_SIGHTINGS are deleted, every device's, so the table never becomes a record of who was where."""
        import alerts
        import camstream
        import devicesettings
        me = facedb.device()["id"]
        camstream.start_heartbeat()                # streaming_at and last_seen, on their own thread
        track = devicesettings.settings("track")
        on = (track.get("ft-o-optAlertMove") or "0") == "1"       # off until switched on
        who = {s.strip().lower() for s in (track.get("ft-alertWho") or "").split(",") if s.strip()}
        now_seen = [n for n in camstream.in_view() if n.lower() in who] if on else []
        conn.execute("DELETE FROM sightings WHERE last_seen < now() - make_interval(secs => %s)", (KEEP_SIGHTINGS,))
        if now_seen:
            conn.cursor().executemany("""INSERT INTO sightings (device, name) VALUES (%s, %s)
                                         ON CONFLICT (device, name) DO UPDATE SET last_seen = now(),
                                         first_seen = CASE WHEN sightings.last_seen < now() - interval '1 minute'
                                                           THEN now() ELSE sightings.first_seen END""",
                                      [(me, n) for n in now_seen])
        arrived = set(now_seen) - self.seen_before
        self.seen_before = set(now_seen)
        for name in sorted(arrived):
            row = conn.execute("""SELECT d.name, s.last_seen FROM sightings s JOIN devices d ON d.id = s.device
                                  WHERE s.name = %s AND s.device <> %s
                                    AND s.last_seen > now() - make_interval(secs => %s)
                                  ORDER BY s.last_seen DESC LIMIT 1""", (name, me, MOVE_WINDOW)).fetchone()
            if row:
                here = facedb.device()["name"]
                try:
                    alerts.add({"kind": "moved", "name": name, "t": int(time.time() * 1000),
                                "text": f"{name} moved from {row[0]}'s camera to {here}'s",
                                "details": {"from": row[0], "to": here,
                                            "secondsBetween": round(max(0.0, time.time() - row[1].timestamp()))}})
                except ValueError:
                    pass

    def _peer(self, conn):
        """The peer service: on while this device shares its photos (peer.sharing), its address in the database."""
        import peer
        peer.CURRENT_KEY = peer.cluster_key(conn)
        want = self.peer_provider is not None and peer.sharing()
        if want and self.peer_service is None:
            try:
                self.peer_service = peer.Service(self.peer_provider, lambda: peer.CURRENT_KEY).start()
                print(f"Sharing photos with the other devices on port {peer.port()}.")
            except OSError as e:
                print(f"Couldn't share photos with the other devices (port {peer.port()}): {e}")
                self.peer_provider = None              # don't try every few seconds
        elif not want and self.peer_service is not None:
            self.peer_service.stop()
            self.peer_service = None
        peer.RUNNING = self.peer_service is not None
        peer.register(conn, peer.RUNNING)

    def after_push(self, conn, state):
        import exprlog
        import face_id as F
        me = facedb.device()["id"]
        exprlog.send(conn, state, me)          # the expression log: readings waiting in HOME/expression
        lp = paths.HOME / "app.log"
        try:
            size = lp.stat().st_size
        except OSError:
            size = 0
        done = int(state.get("log_offset", 0))
        if size < done:                        # rewritten (a person forgotten) or replaced
            done = size
        if size > done:
            with open(lp, "rb") as f:
                f.seek(done)
                chunk = f.read(size - done)
            end = chunk.rfind(b"\n") + 1      # whole lines only; the rest goes next time
            lines = [ln for ln in chunk[:end].decode("utf-8", "replace").splitlines() if ln.strip()]
            if lines:
                now = datetime.now(timezone.utc)
                with conn.transaction():
                    conn.cursor().executemany("INSERT INTO app_log (device, at, line) VALUES (%s, %s, %s)",
                                              [(me, now, ln) for ln in lines])
            state.put("log_offset", done + end)
        if int(state.get("log_pruned_day", 0)) != datetime.now(timezone.utc).toordinal():
            conn.execute("DELETE FROM app_log WHERE device = %s AND at < now() - make_interval(days => %s)",
                         (me, LOG_KEEP_DAYS))
            state.put("log_pruned_day", datetime.now(timezone.utc).toordinal())
        ev = F.DATA / "expr_eval"
        if ev.is_dir():
            rows = []
            for f in sorted(ev.glob("*.jsonl")):
                lines = f.read_text("utf-8").splitlines()
                k0 = int(state.get(f"expr:{f.name}", 0))
                for k in range(min(k0, len(lines)), len(lines)):
                    try:
                        rows.append((f"{f.name}:{k}", me, f.name, json.dumps(json.loads(lines[k]))))
                    except ValueError:
                        continue
                state.put(f"expr:{f.name}", len(lines))
            if rows:
                with conn.transaction():
                    conn.cursor().executemany("INSERT INTO expr_eval (id, device, file, data) VALUES (%s, %s, %s, %s) "
                                              "ON CONFLICT DO NOTHING", rows)
        # Once a day (and after a join): what the server holds about forgotten people is cleared again. A device that
        # was away during a forget sends its log, history and test lines about them when it is back, before it
        # hears of the forget.
        today = datetime.now(timezone.utc).toordinal()
        if int(state.get("forgot_swept_day", 0)) != today:
            names = sorted(forgotten_names())
            if names:
                with conn.transaction():
                    cur = conn.cursor()
                    for name in names:
                        _forget_on_server(cur, name)
            state.put("forgot_swept_day", today)

    def on_join(self, state):
        """After dbimport sent this device's log and tests: only lines written from now on are sent."""
        import face_id as F
        try:
            state.put("log_offset", (paths.HOME / "app.log").stat().st_size)
        except OSError:
            state.put("log_offset", 0)
        ev = F.DATA / "expr_eval"
        for f in sorted(ev.glob("*.jsonl")) if ev.is_dir() else []:
            state.put(f"expr:{f.name}", len(f.read_text("utf-8").splitlines()))


def copy_settings(conn, from_id, part):
    """This device takes another device's page settings (and the describe choices, but its server address, and
    the LM Studio share folder): replaced in the database, then applied here through part (a HomePart).
    Returns how many settings were copied."""
    import uuid
    me = facedb.device()["id"]
    src = str(uuid.UUID(str(from_id)))
    if src == me:
        raise ValueError("That's this device.")
    with conn.transaction():
        n = conn.execute("""SELECT count(*) FROM device_settings WHERE device = %s
                            AND NOT (app = 'describe' AND key = 'url')""", (src,)).fetchone()[0]
        if not n:
            raise ValueError("That device has no settings in the shared database.")
        conn.execute("DELETE FROM device_settings WHERE device = %s AND app IN ('track', 'label')", (me,))
        conn.execute("""INSERT INTO device_settings (device, app, key, value)
                        SELECT %s, app, key, value FROM device_settings
                        WHERE device = %s AND NOT (app = 'describe' AND key = 'url')
                        ON CONFLICT (device, app, key) DO UPDATE SET value = excluded.value""", (me, src))
    keys = part.all_keys(conn, "device_settings")
    gone = [k for k in part._settings() if k.split("|", 2)[1] in ("track", "label") and k not in keys]
    got = part.fetch(conn, "device_settings", keys + gone)
    part.apply("device_settings", got)
    return n


def _utc(v):
    if isinstance(v, dict) and isinstance(v.get("when"), str):
        return dict(v, when=paths.utc_text(v["when"]))
    return v


def _forget_on_server(cur, name):
    """Delete what the server holds about name that no device's files hold (the change log can't carry it)."""
    like = re.escape(name)
    cur.execute("DELETE FROM app_log WHERE line ~* %s", (r"(^|\W)" + like + r"(\W|$)",))
    # (the name inside the faces column is looked for as it is, not as a pattern: "Ann (work)" has brackets)
    cur.execute("DELETE FROM label_history WHERE what ~* %s OR position(%s in faces::text) > 0",
                (r"(^|\W)" + like + r"(\W|$)", '"n": ' + json.dumps(name)))
    cur.execute("UPDATE faces SET fact_name = NULL WHERE fact_name = %s", (name,))
    cur.execute("DELETE FROM expr_eval WHERE name = %s", (name,))
    cur.execute("DELETE FROM person_signals WHERE name = %s", (name,))
    cur.execute("DELETE FROM presence WHERE name = %s", (name,))
    cur.execute("DELETE FROM alerts WHERE name = %s", (name,))
    cur.execute("DELETE FROM sightings WHERE name = %s", (name,))
    cur.execute("DELETE FROM expression_log WHERE name = %s", (name,))

