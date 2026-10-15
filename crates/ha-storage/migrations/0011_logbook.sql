-- Journal de bord : une phrase par chose marquante (« Porte d'entrée s'est
-- ouverte », « Bonne nuit : automatisation lancée »), avec sa cause.
CREATE TABLE logbook (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    at        INTEGER NOT NULL,   -- microsecondes depuis 1970 (UTC)
    entity_id TEXT,               -- NULL : le serveur lui-même, une automatisation
    name      TEXT    NOT NULL,   -- « Porte d'entrée »
    message   TEXT    NOT NULL,   -- « s'est ouverte »
    cause     TEXT                -- « par David », « déclencheur : il est 07:30 »
) STRICT;

CREATE INDEX idx_logbook_at ON logbook (at);
CREATE INDEX idx_logbook_entity ON logbook (entity_id, at);
