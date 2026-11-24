-- The app recognizes faces with a model of its own from here on (Opsis 1, `opsis_1`: prints of 128 numbers). Each
-- device makes its library's prints again and sends them, so the rows change model one device after another;
-- a device takes only the rows of its own program's model.
--
-- A nearest-print index belongs to one model's prints (a print's length is the model's). The ones made for
-- models the app no longer has are dropped, whatever they are called, and this model gets its own.
DO $$
DECLARE
    old record;
BEGIN
    FOR old IN
        SELECT indexname FROM pg_indexes
        WHERE schemaname = current_schema() AND tablename = 'faces' AND indexdef LIKE '%hnsw%'
    LOOP
        EXECUTE format('DROP INDEX %I', old.indexname);
    END LOOP;
END $$;

CREATE INDEX faces_opsis_1 ON faces USING hnsw ((print::vector(128)) vector_cosine_ops)
    WHERE model = 'opsis_1' AND print IS NOT NULL;

INSERT INTO schema_version VALUES (13);
