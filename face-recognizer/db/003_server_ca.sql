-- Version 3: the server's own certificate authority (PEM, public), put here by its administrator
-- (tools/db_certs.py makes it). A device setting up the shared database reads it once over an encrypted connection
-- whose login (SCRAM) also proves the server knows the password, and from then on checks the server with it.

CREATE TABLE server_ca (id int PRIMARY KEY CHECK (id = 1), pem text NOT NULL);

INSERT INTO schema_version VALUES (3);
