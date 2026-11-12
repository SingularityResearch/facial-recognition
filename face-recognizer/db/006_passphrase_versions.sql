-- Version 6: the passphrase for keys, kept in step on every device (secretbox.py).
--   passphrase_versions  every passphrase ever set: its scrypt settings, a known value encrypted with the key made from
--                        it (to tell a right one from a wrong one), the key's fingerprint, who set it and when. One is
--                        current; the others stay, so a wrong change can be undone ("Restore an earlier passphrase").
--   passphrase_shares    each version's passphrase sealed to each device's public key (X25519): only that device can
--                        open it. A device that sets or restores a passphrase seals it to every device, so the others
--                        take it by themselves; nobody types it on each device.
--   devices              each device's public key, and the fingerprints of its passphrase key and database password, so
--                        Settings can show which devices are in step (a fingerprint can't be turned back into either).
-- secret_check (version 1) is replaced by passphrase_versions; it was never used (no passphrase had been set).

ALTER TABLE devices ADD COLUMN public_key bytea;
ALTER TABLE devices ADD COLUMN key_version int;
ALTER TABLE devices ADD COLUMN key_fingerprint text;
ALTER TABLE devices ADD COLUMN password_fingerprint text;

CREATE TABLE passphrase_versions (
    id           serial PRIMARY KEY,
    created_at   timestamptz NOT NULL DEFAULT now(),
    created_by   uuid,
    salt         bytea NOT NULL,
    n            int NOT NULL,
    r            int NOT NULL,
    p            int NOT NULL,
    nonce        bytea NOT NULL,
    ciphertext   bytea NOT NULL,
    fingerprint  text NOT NULL,
    current      boolean NOT NULL DEFAULT false,
    note         text NOT NULL DEFAULT '',            -- "set", "changed", "restored version N"
    updated_at   timestamptz NOT NULL DEFAULT now(),
    updated_by   uuid
);
CREATE UNIQUE INDEX passphrase_versions_one_current ON passphrase_versions (current) WHERE current;

CREATE TABLE passphrase_shares (
    version      int NOT NULL REFERENCES passphrase_versions ON DELETE CASCADE,
    device       uuid NOT NULL REFERENCES devices ON DELETE CASCADE,
    sealed       bytea NOT NULL,                      -- ephemeral public key (32) + nonce (12) + AES-GCM ciphertext
    updated_at   timestamptz NOT NULL DEFAULT now(),
    updated_by   uuid,
    PRIMARY KEY (version, device)
);

CREATE TRIGGER passphrase_versions_touch BEFORE INSERT OR UPDATE ON passphrase_versions FOR EACH ROW
    EXECUTE FUNCTION touch();
CREATE TRIGGER passphrase_versions_change AFTER INSERT OR UPDATE OR DELETE ON passphrase_versions FOR EACH ROW
    EXECUTE FUNCTION note_change('id');
CREATE TRIGGER passphrase_shares_touch BEFORE INSERT OR UPDATE ON passphrase_shares FOR EACH ROW
    EXECUTE FUNCTION touch();
CREATE TRIGGER passphrase_shares_change AFTER INSERT OR UPDATE OR DELETE ON passphrase_shares FOR EACH ROW
    EXECUTE FUNCTION note_change('version', 'device');

DROP TABLE secret_check;

INSERT INTO schema_version VALUES (6);
