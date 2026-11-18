"""
label_settings.py - what the app's Settings tab asks of the labeling tool: LM Studio, this computer, the shared
database and its passphrase, the Web search and describer keys, Music, the downloads it can start.

label_tool.py's request handler hands every request here first (get, post): none of these touch the labeling data,
so none is answered under its lock (a search of the local network, a question to the database or to another
program must not hold up the labeling page, an import or the sync). The labeling page's own routes stay in
label_tool.py. (These were 150 lines among the labeling routes in label_tool.make_handler.)
"""
import sys
import threading

import cudasupport
import describe
import hardware
import lmstudio
import websearch


def get(h, path, db):
    """Answers the GET `path` on the request handler h when it is one of the Settings tab's. Returns False when it
    isn't (nothing was sent); anything else means it was answered."""
    # These ask the network, other programs or files of their own, never the labeling data: outside its
    # lock, or a search of the local network (seconds) held up every labeling request, the import, the
    # sync and the page's own settings request at load.
    if path == "/api/lmstudio":                # LM Studio on this computer, and models it lacks
        st = lmstudio.status()
        return h.send({**st, "missing": lmstudio.missing_models() if st.get("installed") else [],
                       "distributed": hardware.distributed()})
    if path == "/api/hardware/network":       # LM Studio servers on the local network (Settings)
        return h.send({"servers": hardware.lmstudio_network()})
    if path == "/api/db":                      # the shared database: settings (never the password), state
        return h.send(db_state(db))         # asks the database: 0.1 to 0.8 s under the lock before
    if path == "/api/hardware":               # this computer, and where each job runs (Settings)
        return h.send({"has": hardware.describe(), "uses": hardware.uses(), "ai": hardware.ai_apps()})
    if path == "/api/websearch/key":
        return h.send({"set": bool(websearch.key()), "services": websearch.services()})
    if path == "/api/music":                 # the music log and a person's usual music (Settings)
        import musiclog
        return h.send(musiclog.status())
    if path == "/api/secrets":               # the Settings tab's saved keys, shown masked
        if h.refused(write=True):         # only to the tool's own page (its header), never cross-site
            return
        return h.send(secrets())
    return False


def post(h, path, body, db):
    """The same for a POST with its JSON `body`. A ValueError raised here is answered by the caller as a 400, as
    for the labeling routes."""
    if path == "/api/music/spotify":         # Settings: connect Spotify (opens its page in the browser)
        import musiclog
        musiclog.connect_spotify(body.get("client_id") or "")
        return h.send({"ok": True})
    if path == "/api/music/liked":           # Settings: read the liked songs, make the person's picture
        import musiclog
        musiclog.read_liked()
        return h.send({"ok": True})
    if path == "/api/music/disconnect":
        import spotify
        spotify.disconnect()
        return h.send({"ok": True})
    if path == "/api/describe/config":       # Settings: which describer, server, model, API key
        describe.set_config(**{k: body[k] for k in ("use", "url", "model", "key", "spread") if k in body})
        return h.send({"ok": True})
    if path == "/api/describe/test":         # Settings' Test button
        try:
            return h.send({"ok": True, "message": describe.test(body.get("url") or "", body.get("key"))})
        except describe.Unavailable as e:
            raise ValueError(str(e)) from e
    if path == "/api/lmstudio/share":         # the shared models folder (this computer's setting)
        lmstudio.set_share(body.get("folder", ""))
        threading.Thread(target=lmstudio.publish, daemon=True).start()
        return h.send({"ok": True, "shared": lmstudio.share_status()})
    if path == "/api/lmstudio/distributed":   # "Fully distributed system": every adapter, any network
        lmstudio.set_distributed(body.get("on") is True)
        return h.send({"ok": True, "distributed": hardware.distributed()})
    if path == "/api/lmstudio/get":           # download the models other LM Studio servers have
        lmstudio.get_models([m["variant"] for m in lmstudio.missing_models()])
        return h.send({"ok": True})
    if path == "/api/cuda/download":          # NVIDIA CUDA for the Linux app (cudasupport.py), when asked
        cudasupport.start_download()
        return h.send({"ok": True, "status": cudasupport.status()})
    if path == "/api/describe/getmodel":     # download the built-in describer (florence.py), background
        describe.florence.download_in_background()
        return h.send({"ok": True})
    if path == "/api/db/setup":             # set up (or stop using) the shared database
        import facedb
        r = facedb.setup(str(body.get("host") or ""), body.get("port") or 5432,
                         str(body.get("dbname") or "facerec"), str(body.get("user") or ""),
                         str(body.get("password") or ""), str(body.get("schema") or "facerec"),
                         app_version())
        if getattr(db, "sync", None):
            db.sync.note()
        return h.send(dict(r, **db_state(db)))
    if path == "/api/peer":                 # share this device's photos with the other devices
        import peer
        peer.set_sharing(bool(body.get("share")))
        if getattr(db, "sync", None):
            db.sync.note()
        return h.send({"ok": True, "share": peer.sharing()})
    if path == "/api/db/copy":              # this device takes another device's settings
        import dbsync_home
        import facedb
        try:
            with facedb.connect() as conn:
                n = dbsync_home.copy_settings(conn, str(body.get("from") or ""), db.sync.parts[-1])
        except facedb.NotConfigured:             # (answered as a server error before: say what to do instead)
            raise ValueError("Connect to the shared database first.") from None
        return h.send({"ok": True, "copied": n})
    if path in ("/api/db/passphrase", "/api/db/passphrase/restore", "/api/db/passphrase/ask",
                "/api/db/passphrase/approve"):   # passphrase for keys
        import facedb
        import secretbox
        try:
            with facedb.connect() as conn:
                if path.endswith("/restore"):
                    r = secretbox.restore(conn, int(body.get("version") or 0))
                elif path.endswith("/ask"):       # this device asks: a code to type on one that has it
                    r = {"ok": True, "request": secretbox.ask(conn)}
                elif path.endswith("/approve"):   # the code a waiting device shows, typed here
                    r = secretbox.approve(conn, str(body.get("code") or ""),
                                          str(body.get("device") or "") or None)
                else:
                    r = secretbox.set_passphrase(conn, str(body.get("passphrase") or ""),
                                                 str(body.get("again") or ""))
        except facedb.NotConfigured:
            raise ValueError("Connect to the shared database first.") from None
        if getattr(db, "sync", None):
            db.sync.note()
        return h.send(dict(r, **db_state(db)))
    if path == "/api/websearch/aws":        # optional Amazon key, kept outside the project
        websearch.save_aws(body.get("id"), body.get("secret"), body.get("region"))
        return h.send({"ok": True, "services": websearch.services()})
    if path == "/api/websearch/key":        # optional Cloud Vision key, kept outside the project
        websearch.save_key(body.get("key"))
        return h.send({"ok": True, "set": bool(websearch.key()), "services": websearch.services()})
    return False


def app_version():
    """The app's version (app.VERSION), also when app.py runs as __main__ (python app.py, the built app)."""
    for name in ("app", "__main__"):
        v = getattr(sys.modules.get(name), "VERSION", None)
        if isinstance(v, str):
            return v
    return ""


def secrets():
    """Every saved key and password, for the Settings tab to show masked (each can be shown while its eye button is
    held): the Web search keys, the describe server's key, the shared database's password and the passphrase for keys."""
    import describe
    out = {"google": "", "aws": {"id": "", "secret": "", "region": ""}, "describe": describe.config().get("key") or "",
           "db": {}}
    out["google"] = websearch.saved_key()
    a = websearch.aws() or {}
    out["aws"] = {"id": a.get("access_key_id") or "", "secret": a.get("secret_access_key") or "",
                  "region": a.get("region") or ""}
    try:
        import facedb
        import secretbox
        out["db"] = facedb.saved_login() or {}
        out["passphrase"] = secretbox.saved_passphrase()
        out["request"] = secretbox.request()      # this device's one-time code while it asks for the passphrase
    except ImportError:
        out["passphrase"] = ""
    return out


def db_state(db):
    """The shared database for the Settings tab: {available, config (no password), device, sync}."""
    sync = getattr(db, "sync", None)
    if sync is None:
        return {"available": False, "why": "This copy of the app can't use a shared database (psycopg isn't installed)."}
    import facedb
    import secretbox
    import peer
    out = {"available": True, "config": facedb.config(), "device": facedb.device(), "sync": sync.status(),
           "share": peer.sharing(),
           "passphrase": secretbox.status()["here"], "devices": []}
    if out["config"] and out["sync"].get("state") == "online":
        def others(conn):                    # the other devices, for "Copy settings from"
            return dict(devices=[{"id": str(i), "name": n, "seen": s.isoformat().replace("+00:00", "Z")}
                                 for i, n, s in conn.execute("SELECT id, name, last_seen FROM devices "
                                                             "WHERE id <> %s ORDER BY name", (out["device"]["id"],))],
                        keys=secretbox.overview(conn),
                        peers=[{"name": n, "share": bool(s), "addresses": a or [], "port": p}
                               for n, s, a, p in conn.execute("SELECT name, peer_share, peer_addresses, peer_port "
                                                              "FROM devices ORDER BY name")])
        try:                                 # on the connection kept for short questions: Settings asks this every
            out.update(facedb.ask(others))   # 5 s while it is open, and a new connection each time took 0.1 to 0.8 s
        except Exception:
            pass
    return out
