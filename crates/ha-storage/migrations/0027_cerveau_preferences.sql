-- Les préférences de la maison (docs/cerveau.md) : un texte que la famille
-- écrit (« cosy = lumière chaude à 30 % », « on se lève à 7 h »), lu par le
-- cerveau avant de répondre. Une seule ligne (CHECK).
CREATE TABLE cerveau_preferences (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    text       TEXT    NOT NULL,                       -- 2000 caractères au plus, vérifié par l'API
    updated_at INTEGER NOT NULL,                       -- microsecondes depuis 1970 (UTC)
    updated_by TEXT    NOT NULL
) STRICT;
