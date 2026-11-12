-- Version 8: live cameras across devices.
--   devices.streaming_at  when the device's tracker page last sent a frame of its camera (peer.py /stream serves it)
--   sightings             who each device's camera sees now: (device, name, first seen this time, last seen). A device
--                         raises "moved" when someone appears on its camera within MOVE_WINDOW of being seen on
--                         another's (dbsync_home). Names and times only; a person's rows go with a forget.

ALTER TABLE devices ADD COLUMN streaming_at timestamptz;

CREATE TABLE sightings (
    device      uuid NOT NULL REFERENCES devices ON DELETE CASCADE,
    name        text NOT NULL,
    first_seen  timestamptz NOT NULL DEFAULT now(),
    last_seen   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (device, name)
);
CREATE INDEX sightings_name ON sightings (name, last_seen);

INSERT INTO schema_version VALUES (8);
