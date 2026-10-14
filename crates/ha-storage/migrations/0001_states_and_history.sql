-- Première version du schéma.
-- Les dates sont des entiers : microsecondes depuis le 1er janvier 1970 (UTC).
-- Les attributs sont du JSON stocké sous forme de texte.

-- Dernier état connu de chaque entité, pour le restaurer au démarrage.
CREATE TABLE states (
    entity_id    TEXT    PRIMARY KEY NOT NULL,
    state        TEXT    NOT NULL,
    attributes   TEXT    NOT NULL,
    last_changed INTEGER NOT NULL,
    last_updated INTEGER NOT NULL
) STRICT;

-- Historique : une ligne par changement.
-- `state` et `attributes` valent NULL quand l'entité a été supprimée.
CREATE TABLE state_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id    TEXT    NOT NULL,
    state        TEXT,
    attributes   TEXT,
    last_changed INTEGER,
    last_updated INTEGER,
    recorded_at  INTEGER NOT NULL
) STRICT;

-- Pour lire rapidement l'historique d'une entité, et pour purger les vieilles lignes.
CREATE INDEX idx_state_history_entity ON state_history (entity_id, id);
CREATE INDEX idx_state_history_recorded_at ON state_history (recorded_at);
