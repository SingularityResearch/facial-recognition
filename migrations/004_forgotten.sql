-- Version 4: people forgotten on one device (persondata.forget), so every other device forgets them too: its own
-- files (presence log, app.log, undo history, expression tests) and the tracker page's browser storage. The name
-- stays here as the instruction to forget it; nothing else about the person does.

CREATE TABLE forgotten (
    name        text PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  uuid
);
CREATE TRIGGER forgotten_touch BEFORE INSERT OR UPDATE ON forgotten FOR EACH ROW EXECUTE FUNCTION touch();
CREATE TRIGGER forgotten_change AFTER INSERT OR UPDATE OR DELETE ON forgotten FOR EACH ROW
    EXECUTE FUNCTION note_change('name');

INSERT INTO schema_version VALUES (4);
