-- La boîte aux lettres où arrivent les e-mails d'Amazon (étape 87) : la
-- maison y lit, en lecture seule, ceux d'Amazon seulement, pour suivre les
-- colis. Une seule ligne au plus (id = 1). Le mot de passe est un « mot de
-- passe d'application » (Gmail), révocable, chiffré avec ha.key.

CREATE TABLE mail_account (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    host      TEXT    NOT NULL,
    port      INTEGER NOT NULL,
    username  TEXT    NOT NULL,
    password  TEXT    NOT NULL,
    paired_at INTEGER NOT NULL
) STRICT;
