-- Version 16: the signal history: every signal a device reads, kept for the devices whose owner switched that
-- on (the owner, 2026-11-24: "Keep a history of ALL signals in the database, expression readings are kept, music
-- listened to, voice signals, body posture, location, everything we currently get, it is opt-in per device in
-- settings. There needs to be a setting that can adjust how long they are kept for."). Not every reading is a
-- row: a row is every reading's part in one stretch of time (ten seconds), one row for the device itself and
-- one for each named person in its camera's view.
--   name      the person; '' for the device's own row (its camera, its processor and battery, its sound)
--   source    where the readings came from: camera (a device's live camera), or photo | video (worked out from
--             a picture of the library, which says less: no voice, no pulse, no movement; the owner: "We can
--             only assume certain things from the image backfill - and that is OK"). A library's rows have
--             the device 00000000-0000-0000-0000-000000000000: they are the library's, whichever device
--             worked them out, and their time is the picture's own
--   ref       for a library's row the picture it was worked out from, by its content; '' for a camera's row
--   at        when the stretch began (a library's row: when the picture was taken; a video's frame: that
--             many seconds later)
--   seconds   how long the stretch is (0 for a library's row: a picture is one moment)
--   readings  how many readings went into the row
--   data      for every number read in the stretch [average, lowest, highest, how many]; a yes-or-no the same
--             with yes as 1; for a word (the look that won, a heat state, what a sound interface heard) how often
--             each was read; for a person also looks (each look's average share), z (how far from the person's
--             own usual each signal was at its furthest, in spreads) and odd (the most signals unusual at once).
--             A library's row: the face (its permanent id), how good its picture is, how much of the picture
--             it fills, how many faces the picture has and how many of them are named, its looks where the
--             face's picture could be read, and whether the time is the camera's or only the file's
-- A reading says how a face, a voice or a body looked and sounded to the models, never what the person felt,
-- and nothing here is a medical reading. No row names another person: a person's rows are that person's alone,
-- and go with a forget. A face nobody is named for has no row. Who was together is read from the rows
-- themselves: the same device and time, or the same picture.
-- Only ever sent by the devices: not in the change log. Each device deletes its own camera rows that are older
-- than the days its owner set there; a library's rows are as old as its pictures and stay, and follow the
-- library's labels (a picture's rows are made again when they are worked out again).

CREATE TABLE signal_log (
    device    uuid NOT NULL,
    name      text NOT NULL,
    at        timestamptz NOT NULL,
    source    text NOT NULL DEFAULT 'camera' CHECK (source IN ('camera', 'photo', 'video')),
    ref       text NOT NULL DEFAULT '',
    seconds   real NOT NULL CHECK (seconds >= 0),
    readings  integer NOT NULL CHECK (readings > 0),
    data      jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (device, name, at, ref)
);
CREATE INDEX signal_log_name_at ON signal_log (name, at);
CREATE INDEX signal_log_at ON signal_log (at);
CREATE INDEX signal_log_ref ON signal_log (ref) WHERE ref <> '';

INSERT INTO schema_version VALUES (16);
