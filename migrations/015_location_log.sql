-- Version 15: the location history: where a device was, for the devices whose owner switched that on
-- (the owner, 2026-11-24: "add geolocation for the devices", "keeps its location history in the database",
-- "an opt-in setting", and "only keep the On a move for last 90 days"). One row per device and moment. A row
-- is written when the switch is turned on and then only when the device has moved more than about a hundred
-- metres: a device that stays where it is adds nothing.
--   lat, lon  degrees north and east
--   accuracy  metres around that point the device may be, as its system said; null for a place set by hand
--   source    system (the system's own location service) | hand (the owner typed the place on that device)
--   data      short text: place (the nearest town in words, from the list the app carries)
-- Only ever sent by the devices: not in the change log. The rows are about a device, not about a named
-- person. Rows older than 90 days are deleted by the devices, once a day.

CREATE TABLE location_log (
    device    uuid NOT NULL,
    at        timestamptz NOT NULL,
    lat       double precision NOT NULL CHECK (lat BETWEEN -90 AND 90),
    lon       double precision NOT NULL CHECK (lon BETWEEN -180 AND 180),
    accuracy  real CHECK (accuracy IS NULL OR accuracy >= 0),
    source    text NOT NULL CHECK (source IN ('system', 'hand')),
    data      jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (device, at)
);
CREATE INDEX location_log_at ON location_log (at);

INSERT INTO schema_version VALUES (15);
