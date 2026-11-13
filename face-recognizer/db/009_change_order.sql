-- Version 9: the change log says which transaction wrote each change, so a device can tell exactly which changes it
-- has not seen yet.
--   changes.txid   the writing transaction's id (pg_current_xact_id, as a bigint; NULL on rows from before this)
--
-- seq alone isn't enough: a long transaction (a device joining, a large import) takes its seq numbers early and
-- commits late. A device that read the log meanwhile had already moved past those numbers and never saw the rows.
-- dbsync now remembers the snapshot it read the log with, and next time takes every change whose transaction
-- wasn't visible in it (pg_visible_in_snapshot), whatever its seq.

ALTER TABLE changes ADD COLUMN txid bigint;
CREATE INDEX changes_txid ON changes (txid);

CREATE OR REPLACE FUNCTION note_change() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    r jsonb;
    k text := '';
    i int;
BEGIN
    r := to_jsonb(CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END);
    FOR i IN 0 .. TG_NARGS - 1 LOOP
        k := k || CASE WHEN i > 0 THEN '|' ELSE '' END || coalesce(r ->> TG_ARGV[i], '');
    END LOOP;
    INSERT INTO changes (tbl, key, op, device, txid)
    VALUES (TG_TABLE_NAME, k, CASE WHEN TG_OP = 'DELETE' THEN 'D' ELSE 'U' END,
            nullif(current_setting('facerec.device', true), '')::uuid, pg_current_xact_id()::text::bigint);
    PERFORM pg_notify('facerec_changes', TG_TABLE_NAME);
    RETURN NULL;
END $$;

INSERT INTO schema_version VALUES (9);
