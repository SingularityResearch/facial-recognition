-- Face Recognizer shared database, version 1. Run by facedb.migrate() inside the schema named in the connection
-- settings (search_path = <schema>, public), as the role that owns that schema. The vector type comes from the
-- pgvector extension, created once by the database's administrator (CREATE EXTENSION vector).
--
-- Every synced table carries updated_at and updated_by (the device that last changed the row). A trigger writes every
-- insert, update and delete into changes; devices read changes after the last seq they saw, so a delete (Ignore, a
-- forgotten person) reaches them as a tombstone. Keys are the app's own: face IDs and photo hashes come from the photo
-- itself, so the same photo has the same keys on every device.

CREATE TABLE schema_version (version int NOT NULL);

CREATE TABLE devices (
    id           uuid PRIMARY KEY,
    name         text NOT NULL,
    os           text NOT NULL DEFAULT '',
    app_version  text NOT NULL DEFAULT '',
    first_seen   timestamptz NOT NULL DEFAULT now(),
    last_seen    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE changes (
    seq     bigserial PRIMARY KEY,
    tbl     text NOT NULL,
    key     text NOT NULL,
    op      char(1) NOT NULL CHECK (op IN ('U', 'D')),   -- U: inserted or changed (read the row), D: deleted
    device  uuid,
    at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX changes_tbl_key ON changes (tbl, key);

-- The acting device is set per session: SELECT set_config('facerec.device', '<uuid>', false).
CREATE FUNCTION note_change() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    r jsonb;
    k text := '';
    i int;
BEGIN
    r := to_jsonb(CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END);
    FOR i IN 0 .. TG_NARGS - 1 LOOP
        k := k || CASE WHEN i > 0 THEN '|' ELSE '' END || coalesce(r ->> TG_ARGV[i], '');
    END LOOP;
    INSERT INTO changes (tbl, key, op, device)
    VALUES (TG_TABLE_NAME, k, CASE WHEN TG_OP = 'DELETE' THEN 'D' ELSE 'U' END,
            nullif(current_setting('facerec.device', true), '')::uuid);
    PERFORM pg_notify('facerec_changes', TG_TABLE_NAME);
    RETURN NULL;
END $$;

CREATE FUNCTION touch() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := now();
    NEW.updated_by := nullif(current_setting('facerec.device', true), '')::uuid;
    RETURN NEW;
END $$;

-- Settings of one device (each page's options, the describe server address, the LM Studio share folder...).
-- app: track | label | describe | lmstudio | app.
CREATE TABLE device_settings (
    device      uuid NOT NULL REFERENCES devices ON DELETE CASCADE,
    app         text NOT NULL,
    key         text NOT NULL,
    value       jsonb NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid,
    PRIMARY KEY (device, app, key)
);

-- One photo or video, by content (SHA-1 of the file's bytes; legacy keys path-... and nofile-... kept as they were).
CREATE TABLE photos (
    hash        text PRIMARY KEY,
    name        text NOT NULL DEFAULT '',
    size        bigint,
    fp          bytea,                       -- 256-bit difference hash (duplicate detection)
    faces       int,
    dup_of      text,
    video       boolean NOT NULL DEFAULT false,
    fps         real,
    maybe_read  boolean NOT NULL DEFAULT false,   -- read for possible faces
    taken       double precision,                 -- camera date (epoch s), from photo_facts
    tags        text[] NOT NULL DEFAULT '{}',     -- people tags in the photo's metadata (XMP, Google Takeout)
    facts_key   text,
    extra       jsonb NOT NULL DEFAULT '{}',      -- any other field of the photo's record, kept as it came
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

-- Where each device has the photo (drive letters and mount points differ).
CREATE TABLE photo_paths (
    device      uuid NOT NULL REFERENCES devices ON DELETE CASCADE,
    hash        text NOT NULL REFERENCES photos ON DELETE CASCADE,
    path        text NOT NULL,
    mtime       double precision,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid,
    PRIMARY KEY (device, hash)
);

-- One face found in a photo. print is NULL for a printless face (Ignored: only its ID, photo and box stay).
CREATE TABLE faces (
    fid         text PRIMARY KEY,
    photo       text NOT NULL,
    file        text NOT NULL DEFAULT '',
    box         real[] NOT NULL,              -- x, y, w, h, detector score
    quality     real,                         -- eDifFIQA; NULL when not scored
    model       text NOT NULL,                -- recognition model of the print
    print       vector,
    picture     bytea,                        -- 112x112 aligned face, JPEG
    possible    real,                         -- detector score when found as a possible face
    frame       int,                          -- video frame number
    fact_name   text,                         -- the person the photo's own tags put on this face
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE INDEX faces_photo ON faces (photo);
CREATE INDEX faces_arcface ON faces USING hnsw ((print::vector(512)) vector_cosine_ops)
    WHERE model = 'arcface_w600k_r50' AND print IS NOT NULL;
CREATE INDEX faces_sface ON faces USING hnsw ((print::vector(128)) vector_cosine_ops)
    WHERE model = 'sface_2021dec' AND print IS NOT NULL;

-- Each face's label: n a name or #ignore, by me|auto, s the automatic score, ok kept after a warning, nope the names
-- it is not.
CREATE TABLE labels (
    fid         text PRIMARY KEY,
    n           text,
    by          text,
    s           real,
    ok          boolean,
    nope        text[],
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE INDEX labels_n ON labels (n);

CREATE TABLE label_history (
    id      bigserial PRIMARY KEY,
    t       timestamptz NOT NULL,
    device  uuid,
    what    text NOT NULL,
    faces   jsonb NOT NULL
);

CREATE TABLE web_suggestions (
    fid         text PRIMARY KEY,
    data        jsonb NOT NULL,                -- {when, suggestions, checked, details}
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

CREATE TABLE descriptions (
    key         text PRIMARY KEY,              -- photo hash, or hash:frame
    data        jsonb NOT NULL,                -- {text, model, secs, when}
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

-- Collect faces: each saved face (never a whole frame) and the tracker's verdict.
CREATE TABLE webcam_snapshots (
    name        text PRIMARY KEY,              -- webcam-YYYYMMDD-HHMMSS-mmm[-k]
    device      uuid,
    taken       timestamptz,
    jpeg        bytea NOT NULL,
    verdict     jsonb,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

CREATE TABLE alerts (
    id          text PRIMARY KEY,
    device      uuid,
    data        jsonb NOT NULL,                -- {id, t, kind, name, text, state, details}
    name        text GENERATED ALWAYS AS (data ->> 'name') STORED,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE INDEX alerts_name ON alerts (name);

CREATE TABLE presence (
    id          text PRIMARY KEY,              -- device:start:name, so a visit sent twice is kept once
    device      uuid,
    name        text NOT NULL,
    start_ms    bigint NOT NULL,
    end_ms      bigint NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE INDEX presence_name ON presence (name);

-- Per person: kind usual (usual look) or baseline (each signal's normal); data is the browser's own shape.
CREATE TABLE person_signals (
    name        text NOT NULL,
    kind        text NOT NULL,
    data        jsonb NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid,
    PRIMARY KEY (name, kind)
);

-- Opt-in posed expression test sessions and feeling reports (numbers only), one row per line.
CREATE TABLE expr_eval (
    id          text PRIMARY KEY,              -- file:line
    device      uuid,
    file        text NOT NULL,
    data        jsonb NOT NULL,
    name        text GENERATED ALWAYS AS (data ->> 'name') STORED,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE INDEX expr_eval_name ON expr_eval (name);

-- Whole small documents: expr_calibration, va_fit, model_eval, hw_bench, webcam_eval, photo_folders...
-- device NULL: shared; otherwise that device's own.
CREATE TABLE documents (
    id          text PRIMARY KEY,              -- name, or device:name
    device      uuid,
    name        text NOT NULL,
    body        jsonb NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

-- Web search use per month, shared so each service's cap holds across devices.
CREATE TABLE usage (
    service     text NOT NULL,
    month       text NOT NULL,
    used        int NOT NULL DEFAULT 0,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid,
    PRIMARY KEY (service, month)
);

-- Keys, encrypted on the device (AES-256-GCM, key from the owner's passphrase by scrypt). The server never sees them
-- in clear. secret_check holds the scrypt settings and a known value encrypted, to check a passphrase.
CREATE TABLE secrets (
    name        text PRIMARY KEY,
    nonce       bytea NOT NULL,
    ciphertext  bytea NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE TABLE secret_check (
    id          int PRIMARY KEY CHECK (id = 1),
    salt        bytea NOT NULL,
    n           int NOT NULL,
    r           int NOT NULL,
    p           int NOT NULL,
    nonce       bytea NOT NULL,
    ciphertext  bytea NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);

CREATE TABLE app_log (
    id      bigserial PRIMARY KEY,
    device  uuid,
    at      timestamptz NOT NULL DEFAULT now(),
    line    text NOT NULL
);
CREATE INDEX app_log_at ON app_log (at);

-- Triggers: touch() keeps updated_at/updated_by; note_change() records the change, keyed as each device keys the row.
DO $$
DECLARE
    t record;
BEGIN
    FOR t IN SELECT * FROM (VALUES
        ('device_settings', 'device,app,key'), ('photos', 'hash'), ('photo_paths', 'device,hash'),
        ('faces', 'fid'), ('labels', 'fid'), ('web_suggestions', 'fid'), ('descriptions', 'key'),
        ('webcam_snapshots', 'name'), ('alerts', 'id'), ('presence', 'id'), ('person_signals', 'name,kind'),
        ('expr_eval', 'id'), ('documents', 'id'), ('usage', 'service,month'), ('secrets', 'name'),
        ('secret_check', 'id')) AS v(tbl, keys)
    LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE INSERT OR UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION touch()',
                       t.tbl || '_touch', t.tbl);
        EXECUTE format('CREATE TRIGGER %I AFTER INSERT OR UPDATE OR DELETE ON %I FOR EACH ROW '
                       'EXECUTE FUNCTION note_change(%s)', t.tbl || '_change', t.tbl,
                       (SELECT string_agg(quote_literal(k), ',') FROM unnest(string_to_array(t.keys, ',')) k));
    END LOOP;
END $$;

INSERT INTO schema_version VALUES (1);
