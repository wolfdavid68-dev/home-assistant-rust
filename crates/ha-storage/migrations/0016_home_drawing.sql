-- Le dessin de la maison, au centre du style « plan d'architecte » du
-- tableau de bord (étape 36) : des images (le fond de jour, celui de nuit,
-- des calques transparents), et la façon de les assembler.

-- Les images, vérifiées avant d'être rangées (PNG, JPEG ou WebP).
CREATE TABLE home_images (
    name         TEXT    PRIMARY KEY,                  -- [a-z0-9_-], ex. garage_ouvert
    content_type TEXT    NOT NULL,
    data         BLOB    NOT NULL,
    updated_at   INTEGER NOT NULL,                     -- microsecondes depuis 1970 (UTC)
    updated_by   TEXT    NOT NULL
) STRICT;

-- L'assemblage : quel fond, quels calques selon quels états, quelles valeurs
-- posées sur le dessin. Une seule ligne (CHECK).
CREATE TABLE home_drawing (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    config     TEXT    NOT NULL,                       -- JSON, vérifié par l'API
    updated_at INTEGER NOT NULL,
    updated_by TEXT    NOT NULL
) STRICT;
