-- La maison SmartThings de Samsung (étape 86). Une seule ligne au plus
-- (id = 1) : l'application SmartThings créée pour la maison (son
-- identifiant et son secret), et les jetons qu'elle a reçus.
--
-- Le jeton d'accès vit 24 heures ; le jeton de renouvellement ne sert
-- qu'une fois et doit être utilisé avant 30 jours. Les deux sont remplacés
-- ensemble, à chaque renouvellement. Les secrets sont chiffrés avec ha.key.

CREATE TABLE smartthings_account (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    client_id     TEXT    NOT NULL,
    client_secret TEXT    NOT NULL,
    redirect_uri  TEXT    NOT NULL,
    access_token  TEXT    NOT NULL,
    refresh_token TEXT    NOT NULL,
    -- Fin de validité du jeton d'accès, en microsecondes depuis 1970.
    expires_at    INTEGER NOT NULL,
    refreshed_at  INTEGER NOT NULL,
    paired_at     INTEGER NOT NULL
) STRICT;
