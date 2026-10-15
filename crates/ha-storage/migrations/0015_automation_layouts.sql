-- La place des blocs de chaque automatisation dans le schéma de l'éditeur
-- (étape 35). Purement visuel : la configuration, elle, reste dans
-- `automations` (ou dans le fichier). Gardée à part pour qu'un simple
-- déplacement de bloc ne crée pas de nouvelle version.
CREATE TABLE automation_layouts (
    automation_id TEXT    PRIMARY KEY,
    layout        TEXT    NOT NULL,                    -- JSON : { "1 › alors › 2": [x, y], … }
    updated_at    INTEGER NOT NULL                     -- microsecondes depuis 1970 (UTC)
) STRICT;
