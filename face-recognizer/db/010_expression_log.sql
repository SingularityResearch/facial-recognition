-- Version 10: the expression log (exprlog.py): what each named person's face looked like, every few seconds while a
-- tracker saw them, kept for reports. One row per device, person and moment. A reading is what the face looked like
-- to the model, never what the person felt.
--   label  the expression shown            share  its share of the last second's votes, 0..1
--   state  plain | change | usual | unsure (departs from / matches the person's usual look; no clear winner)
--   data   the rest, numbers and short text: lead, votes, scores (the latest frame's score for every expression),
--          valence, arousal, flags, setting (alone | company), match, quality
-- Only ever sent by the devices and read by reports: not in the change log. A person's rows go with a forget.

CREATE TABLE expression_log (
    device  uuid NOT NULL,
    name    text NOT NULL,
    at      timestamptz NOT NULL,
    label   text NOT NULL,
    share   real,
    state   text NOT NULL DEFAULT '',
    data    jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (device, name, at)
);
CREATE INDEX expression_log_name_at ON expression_log (name, at);
CREATE INDEX expression_log_at ON expression_log (at);

INSERT INTO schema_version VALUES (10);
