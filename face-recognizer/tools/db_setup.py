"""Prepare a new shared database for Face Recognizer devices, once, after its administrator made the role and schema
and the server has its certificates (tools/db_certs.py): makes the tables (every db/NNN_*.sql) and stores the
certificate authority in server_ca, which each device's Settings → Shared database → Connect then takes.

    python tools/db_setup.py --host nas.local --dbname <db> --ca <folder>/ca.crt [--port 5432] [--user facerec]

The password is asked for (or FACE_DB_PASSWORD). The connection checks the server against ca.crt (verify-ca), so the
tables and the certificate go only to the server that certificate signed. Run again any time: it only adds what's
missing (a newer version's tables, or a changed ca.crt)."""
import argparse
import getpass
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import facedb  # noqa: E402


def main(argv=None):
    a = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    a.add_argument("--host", required=True)
    a.add_argument("--port", type=int, default=5432)
    a.add_argument("--dbname", required=True)
    a.add_argument("--user", default="facerec")
    a.add_argument("--schema", default="facerec")
    a.add_argument("--ca", required=True, help="the ca.crt that tools/db_certs.py made")
    o = a.parse_args(argv)
    ca = Path(o.ca)
    pem = ca.read_text("utf-8")
    if "BEGIN CERTIFICATE" not in pem or "PRIVATE KEY" in pem:
        raise SystemExit(f"{ca} isn't a certificate (use ca.crt, never a .key)")
    if not facedb.SCHEMA_RE.match(o.schema):
        raise SystemExit("The schema name may have only lower-case letters, digits and _.")
    password = os.environ.get("FACE_DB_PASSWORD") or getpass.getpass(f"Password of {o.user}: ")
    import psycopg
    from psycopg.conninfo import make_conninfo
    info = make_conninfo(host=o.host, port=o.port, dbname=o.dbname, user=o.user, password=password,
                         sslmode="verify-ca", sslrootcert=str(ca.resolve()), connect_timeout=15,
                         application_name="Face Recognizer setup")
    with psycopg.connect(info, autocommit=True) as conn:
        conn.execute(f"SET search_path = {o.schema}, public")
        conn.execute("SET TIME ZONE 'UTC'")
        done = facedb.migrate(conn)
        conn.execute("""INSERT INTO server_ca (id, pem) VALUES (1, %s)
                        ON CONFLICT (id) DO UPDATE SET pem = excluded.pem""", (pem,))
        print(f"Tables: {'made ' + ', '.join(map(str, done)) if done else 'already up to date'} "
              f"(version {facedb.version(conn)}). Certificate authority stored.")
    print("Now on each device: Settings > Shared database > Connect.")


if __name__ == "__main__":
    main()
