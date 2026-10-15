-- Statistiques long terme (étape 24) : un résumé par heure et par capteur,
-- gardé des années (l'historique détaillé, lui, est purgé après quelques jours).
CREATE TABLE statistics (
    entity_id TEXT    NOT NULL,
    start     INTEGER NOT NULL,   -- début de l'heure, microsecondes depuis 1970 (UTC)
    mean      REAL    NOT NULL,   -- moyenne pondérée par la durée
    min       REAL    NOT NULL,
    max       REAL    NOT NULL,
    last      REAL    NOT NULL,   -- dernière valeur de l'heure
    change    REAL,               -- compteurs seulement : consommation de l'heure
    PRIMARY KEY (entity_id, start)
) STRICT, WITHOUT ROWID;

-- Les heures déjà calculées : au démarrage, on rattrape celles qui manquent.
CREATE TABLE statistics_runs (
    start INTEGER PRIMARY KEY NOT NULL
) STRICT;
