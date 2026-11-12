-- Version 2: what the sync (dbsync.py) needs besides the tables of version 1.
--   instance  one id for this database: a device remembers which database it joined, and joins again when it's
--             pointed at another one
--   photos.facts  what the photo says about itself, as the device that read it saw it: {t, tags, faces: {fid: name}}
--             (camera date, people tags). taken and tags stay as they were for searching.

CREATE TABLE instance (id uuid PRIMARY KEY);
INSERT INTO instance VALUES (gen_random_uuid());

ALTER TABLE photos ADD COLUMN facts jsonb;
UPDATE photos p SET facts = jsonb_build_object(
    't', p.taken, 'tags', to_jsonb(p.tags),
    'faces', coalesce((SELECT jsonb_object_agg(f.fid, f.fact_name) FROM faces f
                       WHERE f.photo = p.hash AND f.fact_name IS NOT NULL), '{}'::jsonb))
WHERE p.taken IS NOT NULL OR cardinality(p.tags) > 0 OR p.facts_key IS NOT NULL;

INSERT INTO schema_version VALUES (2);
