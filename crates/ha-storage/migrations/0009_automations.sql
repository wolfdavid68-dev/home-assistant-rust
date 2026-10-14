-- Automatisations créées ou modifiées depuis le tableau de bord (étape 13).
-- Celles de `automations.yaml` restent dans le fichier, en lecture seule.

-- Une ligne par automatisation : sa configuration, en JSON, telle qu'elle a
-- été vérifiée avant l'enregistrement. L'ordre d'affichage est celui de la
-- création (`position`).
CREATE TABLE automations (
    id         TEXT    PRIMARY KEY,
    position   INTEGER NOT NULL,
    config     TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    updated_by TEXT    NOT NULL
) STRICT;

-- Les versions précédentes, pour revenir en arrière après une erreur.
-- `config` vaut NULL pour une suppression.
CREATE TABLE automation_versions (
    version       INTEGER PRIMARY KEY AUTOINCREMENT,
    automation_id TEXT    NOT NULL,
    config        TEXT,
    changed_at    INTEGER NOT NULL,
    changed_by    TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_automation_versions_id ON automation_versions (automation_id, version);
