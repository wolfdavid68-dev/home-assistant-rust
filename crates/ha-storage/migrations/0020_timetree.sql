-- Le compte TimeTree qui lit l'agenda de la famille (étape 74). Une seule
-- ligne au plus (id = 1). C'est un compte dédié à la maison, invité dans
-- l'agenda partagé : jamais le compte personnel.

CREATE TABLE timetree_account (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    email     TEXT    NOT NULL,
    -- Le mot de passe, chiffré avec la clé ha.key (TimeTree n'a pas de jeton
    -- d'accès : il faut se reconnecter quand la session expire).
    password  TEXT    NOT NULL,
    paired_at INTEGER NOT NULL
) STRICT;
