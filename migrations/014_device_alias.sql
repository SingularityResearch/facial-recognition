-- Version 14: a name the owner gives a device, by its id (the owner, 2026-11-23: "We need to be able to create
-- device aliases (for a device id)", "in the UI", and "unique constraint on device name - similar to an email
-- account"). A phone calls itself "iPhone", and two of them are told apart by nothing else. The alias is
-- shared: every device shows that device under it, wherever it shows a device's name. The device's own name
-- stays what the device says of itself; an empty alias means none.
--
-- Any device of the database may set it (Settings, Shared database): it is what the owner calls a device, like
-- a label, not a setting of that device. It is shown as text and never used for anything else.
--
-- No two devices have the same alias, whatever its capitals and the spaces around it: the index holds that
-- whoever writes the row. (That an alias is not another device's own name either is checked where it is set,
-- `face_db::set_alias`: a device's own name is the device's to say, and two phones may both say "iPhone".)

ALTER TABLE devices ADD COLUMN alias text NOT NULL DEFAULT '';
CREATE UNIQUE INDEX devices_alias_unique ON devices (lower(btrim(alias))) WHERE btrim(alias) <> '';

INSERT INTO schema_version VALUES (14);
