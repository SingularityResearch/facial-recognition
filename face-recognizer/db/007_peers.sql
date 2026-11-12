-- Version 7: devices show each other their original photos (peer.py). Each device that shares records where it can
-- be reached on the local network and its own TLS certificate's SHA-256, which the asking device checks.

ALTER TABLE devices ADD COLUMN peer_addresses text[];
ALTER TABLE devices ADD COLUMN peer_port int;
ALTER TABLE devices ADD COLUMN peer_cert_sha256 text;
ALTER TABLE devices ADD COLUMN peer_share boolean NOT NULL DEFAULT false;

INSERT INTO schema_version VALUES (7);
