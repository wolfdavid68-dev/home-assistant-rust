-- Les automatisations toutes prêtes (étape 85) : ce qui a été coché pour
-- chacune (« éteindre les lumières », « chauffage à 17 °C »…). L'automatisation
-- elle-même est rangée avec les autres (table automations, id « pret_… ») ;
-- ces réglages servent à la refabriquer et à remplir le formulaire.
-- (0022 est pris par la liste de courses d'Iglou, étape 81.)

CREATE TABLE automation_presets (
    name       TEXT    PRIMARY KEY NOT NULL,
    -- Les réglages, en JSON (vérifiés par ha-automation avant d'arriver ici).
    options    TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    updated_by TEXT    NOT NULL
) STRICT;
