-- Version 5: each face picture's SHA-256, kept by the server itself. The sync compares it with the device's own copy
-- and sends or fetches a picture only when they differ (a face's other fields change far more often than its picture).

ALTER TABLE faces ADD COLUMN picture_sha bytea GENERATED ALWAYS AS (sha256(picture)) STORED;

INSERT INTO schema_version VALUES (5);
