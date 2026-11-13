-- Version 11: the passphrase for keys goes only to devices the owner let in (secretbox.py). Until now it was sealed
-- to every row in devices with a public key, so whoever held the database password could add a row and be sent it.
--   devices.key_proof          a keyed check value only a device holding the current passphrase can make, over its
--                              own id and public key: a new passphrase is sealed only to devices that show it.
--   passphrase_versions.prev   the version that was current when this one was made, and
--   passphrase_versions.link   a keyed check value made with that version's key: a device follows a change only when
--                              it was made by a device holding the passphrase it has itself.
--   key_requests               a device without the passphrase asks for it: it shows a one-time code on its own screen
--                              and puts here a check value made from that code. The owner types the code on a device
--                              that has the passphrase, which answers here: the passphrase sealed to the asking device,
--                              with a check value made from the same code. A request lasts 15 minutes; the code itself
--                              is never here.

ALTER TABLE devices ADD COLUMN key_proof bytea;
ALTER TABLE passphrase_versions ADD COLUMN prev int;
ALTER TABLE passphrase_versions ADD COLUMN link bytea;

CREATE TABLE key_requests (
    device       uuid PRIMARY KEY REFERENCES devices ON DELETE CASCADE,
    public_key   bytea NOT NULL,
    salt         bytea NOT NULL,
    mac          bytea NOT NULL,
    asked_at     timestamptz NOT NULL DEFAULT now(),
    version      int,                                 -- the answer: the version, its passphrase sealed to public_key,
    sealed       bytea,                               -- and the check value made from the code
    answer_mac   bytea,
    answered_by  uuid,
    answered_at  timestamptz
);

INSERT INTO schema_version VALUES (11);
