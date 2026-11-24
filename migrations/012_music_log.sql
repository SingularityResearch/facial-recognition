-- Version 12: the music log: which song played on a device, when, for whom, and how it
-- sounded on each of that device's audio interfaces. One row per device, person and the moment the song was first
-- seen playing. The numbers describe the sound (tempo, loudness, brightness, major or minor), never what the
-- listener felt.
--   name     the person whose music it is: the device's "This device's music belongs to" (never Unknown)
--   artist   '' when no player named the song: music an interface heard ("music in the room")
--   seconds  how long the device saw or heard it playing
--   data     numbers and short text: the app that showed it (or "room"), "interfaces" (each audio interface that
--            wasn't quiet: kind output | input, what it heard in a word and how sure, the families of sound, the
--            background's numbers, and a song's numbers where music sounded), "heard" (the numbers taken for
--            the song: from an output before a microphone), "place" (the song against the person's usual music)
-- A person's usual music itself (music.picture) is a person signal of kind 'music' (person_signals, version 1).
-- Only ever sent by the devices and read by reports: not in the change log. A person's rows go with a forget.

CREATE TABLE music_log (
    device   uuid NOT NULL,
    name     text NOT NULL,
    at       timestamptz NOT NULL,
    artist   text NOT NULL,
    song     text NOT NULL,
    seconds  real,
    data     jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (device, name, at)
);
CREATE INDEX music_log_name_at ON music_log (name, at);
CREATE INDEX music_log_at ON music_log (at);

INSERT INTO schema_version VALUES (12);
