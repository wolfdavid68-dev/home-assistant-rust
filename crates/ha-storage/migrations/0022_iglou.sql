-- Le compte Iglou de la maison (étape 81) : la liste de courses du frigo et du
-- congélateur. Une seule ligne au plus (id = 1). C'est un compte dédié à la
-- maison, membre de la maison Iglou : jamais un compte personnel.

CREATE TABLE iglou_account (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    -- Le projet Firebase de l'appli, et sa clé « web » (publique : elle est
    -- dans l'appli elle-même, elle ne donne aucun droit sans compte).
    project_id TEXT    NOT NULL,
    api_key    TEXT    NOT NULL,
    email      TEXT    NOT NULL,
    -- Le mot de passe, chiffré avec la clé ha.key : il faut se reconnecter
    -- quand la session (une heure) expire.
    password   TEXT    NOT NULL,
    -- La maison Iglou dont la liste est lue.
    maison_id  TEXT    NOT NULL,
    paired_at  INTEGER NOT NULL
) STRICT;
