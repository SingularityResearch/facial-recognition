"""The shared database (optional): PostgreSQL with pgvector, on a server every Face Recognizer device can reach.

Without one configured, nothing here is used and every program keeps its data in files as before. With one, this
module connects (TLS, checked against the server's own certificate authority), brings the schema up to date from
db/NNN_*.sql, and registers this device. dbsync.py copies the data both ways.

Where the settings live (never in data/ or git):
  HOME/database.json   {host, port, dbname, user, schema, password (protected), ca (PEM of the server's CA)}
  HOME/device.json     {id, name}: this device, made once
FACE_DB (a libpq connection string, e.g. "host=... dbname=... user=... password=...") and FACE_DB_SCHEMA override
the file; tests use them.

The password is protected with Windows DPAPI (only this Windows user on this computer can read it back); on macOS
and Linux the file is readable by its owner only."""
import base64
import json
import os
import platform
import re
import socket
import sys
import threading
import uuid
from pathlib import Path

import paths

CONFIG = paths.HOME / "database.json"
DEVICE = paths.HOME / "device.json"
SQL_DIR = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent)) / "db"
SCHEMA_RE = re.compile(r"^[a-z_][a-z0-9_]{0,62}$")
CONNECT_TIMEOUT = 5            # seconds: an unreachable server must not hold up the programs


class NotConfigured(Exception):
    """No shared database is set up on this device."""


# --- protecting the password on disk ---------------------------------------------------------------------------

def _dpapi(data, protect):
    import ctypes
    from ctypes import wintypes

    class Blob(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]

    buf = ctypes.create_string_buffer(data, len(data))
    src, out = Blob(len(data), buf), Blob()
    fn = ctypes.windll.crypt32.CryptProtectData if protect else ctypes.windll.crypt32.CryptUnprotectData
    if not fn(ctypes.byref(src), None, None, None, None, 0x1, ctypes.byref(out)):   # 0x1: no UI
        raise OSError("Windows couldn't " + ("protect" if protect else "read") + " the saved database password")
    try:
        return ctypes.string_at(out.pbData, out.cbData)
    finally:
        ctypes.windll.kernel32.LocalFree(out.pbData)


def protect(text):
    """A string safe to keep in a file for this user: DPAPI on Windows, plain elsewhere (the file is mode 600)."""
    raw = text.encode()
    if sys.platform == "win32":
        return "dpapi:" + base64.b64encode(_dpapi(raw, True)).decode()
    return "plain:" + base64.b64encode(raw).decode()


def unprotect(stored):
    kind, _, b64 = (stored or "").partition(":")
    raw = base64.b64decode(b64)
    if kind == "dpapi":
        return _dpapi(raw, False).decode()
    if kind == "plain":
        return raw.decode()
    raise ValueError("unknown password format")


def write_private(path, obj):
    """Write JSON readable only by this user (mode 600 outside Windows; HOME is per user on Windows)."""
    data = json.dumps(obj, indent=1).encode()
    paths.atomic_write(path, data)
    if sys.platform != "win32":
        os.chmod(path, 0o600)


# --- settings ----------------------------------------------------------------------------------------------------

def config():
    """This device's database settings, or None when there is no shared database. Never includes the password."""
    if os.environ.get("FACE_DB"):
        return {"env": True, "schema": os.environ.get("FACE_DB_SCHEMA", "facerec")}
    try:
        c = json.loads(CONFIG.read_text("utf-8"))
    except (OSError, ValueError):
        return None
    if not c.get("host"):
        return None
    return {k: c.get(k) for k in ("host", "port", "dbname", "user", "schema")} | {"ca": bool(c.get("ca"))}


def configured():
    return config() is not None


def save_config(host, port, dbname, user, password, ca_pem, schema="facerec"):
    """Keep the connection settings (the password protected). An empty host removes them: back to files only."""
    if not host:
        try:
            CONFIG.unlink()
        except FileNotFoundError:
            pass
        return None
    if not SCHEMA_RE.match(schema or ""):
        raise ValueError("The schema name may have only lower-case letters, digits and _.")
    if ca_pem and "BEGIN CERTIFICATE" not in ca_pem:
        raise ValueError("The certificate authority must be a PEM certificate (-----BEGIN CERTIFICATE-----).")
    write_private(CONFIG, {"host": host.strip(), "port": int(port or 5432), "dbname": dbname.strip(),
                           "user": user.strip(), "schema": schema, "password": protect(password),
                           "ca": ca_pem or ""})
    return config()


def schema():
    c = config()
    if c is None:
        raise NotConfigured()
    s = c.get("schema") or "facerec"
    if not SCHEMA_RE.match(s):
        raise ValueError("bad schema name")
    return s


def _conninfo():
    """(libpq connection string, schema)."""
    if os.environ.get("FACE_DB"):
        return os.environ["FACE_DB"], schema()
    try:
        c = json.loads(CONFIG.read_text("utf-8"))
    except (OSError, ValueError):
        raise NotConfigured() from None
    if not c.get("host"):
        raise NotConfigured()
    from psycopg.conninfo import make_conninfo
    kw = dict(host=c["host"], port=int(c.get("port") or 5432), dbname=c.get("dbname") or "postgres",
              user=c.get("user") or "", password=unprotect(c.get("password")), connect_timeout=CONNECT_TIMEOUT,
              application_name="Face Recognizer")
    if c.get("ca"):
        ca = paths.HOME / "database-ca.pem"
        if not ca.exists() or ca.read_text("utf-8") != c["ca"]:
            paths.atomic_write(ca, c["ca"].encode())
        # verify-ca: the server must show a certificate from this authority. (Not verify-full: a device may use any
        # of the server's addresses; the authority is this server's own, made by tools/db_certs.py.)
        kw |= dict(sslmode="verify-ca", sslrootcert=str(ca))
    else:
        kw |= dict(sslmode="require")
    return make_conninfo(**kw), c.get("schema") or "facerec"


# --- this device -------------------------------------------------------------------------------------------------

_device_lock = threading.Lock()


def device():
    """{id, name} of this device, made the first time it's asked for."""
    with _device_lock:
        try:
            d = json.loads(DEVICE.read_text("utf-8"))
            uuid.UUID(d["id"])
            return d
        except (OSError, ValueError, KeyError, TypeError):
            pass
        d = {"id": str(uuid.uuid4()), "name": socket.gethostname() or "this device"}
        paths.HOME.mkdir(parents=True, exist_ok=True)
        paths.atomic_write(DEVICE, json.dumps(d).encode())
        return d


# --- connections -------------------------------------------------------------------------------------------------

def connect(autocommit=True):
    """A new connection with search_path = <schema>, public and this device set for the change log. Autocommit by
    default: group statements with `with conn.transaction():` (without autocommit that would only be a savepoint
    inside a transaction nobody commits).
    Raises NotConfigured, or psycopg.OperationalError when the server can't be reached."""
    import psycopg
    info, sch = _conninfo()
    if not SCHEMA_RE.match(sch):
        raise ValueError("bad schema name")
    conn = psycopg.connect(info, autocommit=True)
    try:
        conn.execute(f"SET search_path = {sch}, public")
        conn.execute("SET TIME ZONE 'UTC'")       # times are kept and exchanged in UTC
        conn.execute("SELECT set_config('facerec.device', %s, false)", (device()["id"],))
        try:                                    # the vector type for numpy arrays, when pgvector's helper is there
            from pgvector.psycopg import register_vector
            register_vector(conn)
        except Exception:
            pass
        conn.autocommit = autocommit
        return conn
    except Exception:
        conn.close()
        raise


def migrations():
    """[(number, path)] of db/NNN_*.sql, in order."""
    out = []
    for p in sorted(SQL_DIR.glob("[0-9][0-9][0-9]_*.sql")):
        out.append((int(p.name[:3]), p))
    return out


def version(conn):
    row = conn.execute("SELECT to_regclass('schema_version') IS NOT NULL").fetchone()
    if not row[0]:
        return 0
    row = conn.execute("SELECT max(version) FROM schema_version").fetchone()
    return row[0] or 0


def migrate(conn):
    """Bring the schema up to the newest db/NNN_*.sql, each file in one transaction (under an advisory lock, so two
    devices starting together don't both run one). Returns the numbers applied."""
    done = []
    with conn.transaction():
        conn.execute("SELECT pg_advisory_xact_lock(hashtext(current_schema()))")
        have = version(conn)
        for n, p in migrations():
            if n > have:
                conn.execute(p.read_text("utf-8"))
                done.append(n)
    return done


def register(conn, app_version=""):
    """Note this device (name, system, app version, last seen)."""
    d = device()
    conn.execute("""INSERT INTO devices (id, name, os, app_version) VALUES (%s, %s, %s, %s)
                    ON CONFLICT (id) DO UPDATE SET name = excluded.name, os = excluded.os,
                    app_version = excluded.app_version, last_seen = now()""",
                 (d["id"], d["name"], f"{platform.system()} {platform.release()}", app_version))
    return d


def open_ready(app_version=""):
    """Connect, migrate and register: the one call a program makes at start. Returns the connection."""
    conn = connect()
    try:
        migrate(conn)
        with conn.transaction():
            register(conn, app_version)
        return conn
    except Exception:
        conn.close()
        raise


def setup(host, port, dbname, user, password, schema="facerec", app_version=""):
    """Set up the shared database from the Settings tab: log in over an encrypted connection (sslmode=require; the
    SCRAM login proves the server knows this password), take the server's certificate authority from its server_ca
    table, then keep the settings with it so every later connection checks the server (verify-ca). Returns test()."""
    import psycopg
    if not host:
        save_config("", 0, "", "", "", "")
        return {"ok": True, "text": "The shared database is no longer used: data is kept on this device only."}
    if not SCHEMA_RE.match(schema or ""):
        raise ValueError("The schema name may have only lower-case letters, digits and _.")
    from psycopg.conninfo import make_conninfo
    info = make_conninfo(host=host.strip(), port=int(port or 5432), dbname=(dbname or "").strip(),
                         user=(user or "").strip(), password=password or "", sslmode="require",
                         connect_timeout=CONNECT_TIMEOUT, application_name="Face Recognizer")
    try:
        with psycopg.connect(info, autocommit=True) as conn:
            conn.execute(f"SET search_path = {schema}, public")
            ok = conn.execute("SELECT to_regclass('server_ca') IS NOT NULL").fetchone()[0]
            row = conn.execute("SELECT pem FROM server_ca WHERE id = 1").fetchone() if ok else None
    except psycopg.Error as e:
        return {"ok": False, "text": f"Couldn't log in: {str(e).strip().splitlines()[0][:300]}"}
    if not row:
        return {"ok": False, "text": "The server has no certificate authority in its server_ca table: ask its "
                                     "administrator to add it (tools/db_certs.py), so this device can check the server."}
    save_config(host, port, dbname, user, password, row[0], schema)
    return test(app_version)


def test(app_version=""):
    """{ok, text} for the Settings tab's Test button."""
    try:
        conn = open_ready(app_version)
    except NotConfigured:
        return {"ok": False, "text": "No shared database is set up."}
    except Exception as e:
        return {"ok": False, "text": f"Couldn't use the database: {str(e).strip().splitlines()[0][:300]}"}
    try:
        row = conn.execute("SELECT version(), (SELECT extversion FROM pg_extension WHERE extname = 'vector'),"
                           " current_schema(), (SELECT count(*) FROM devices)").fetchone()
        return {"ok": True, "text": f"Connected: {row[0].split(' on ')[0]}, pgvector {row[1]}, schema {row[2]}, "
                                    f"{row[3]} device(s)."}
    finally:
        conn.close()
