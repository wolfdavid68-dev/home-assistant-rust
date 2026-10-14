-- Association avec une box Somfy TaHoma (API locale, « mode développeur »).
-- Une seule ligne au plus (id = 1).

CREATE TABLE somfy_box (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    host        TEXT    NOT NULL,
    gateway_id  TEXT    NOT NULL,
    -- Empreinte SHA-256 du certificat de la box, mémorisée à l'association.
    cert_sha256 BLOB    NOT NULL,
    -- Jeton de l'API locale. C'est un secret : il permet de piloter les
    -- volets et de lire l'état de l'alarme. La base est lisible par toi seul.
    token       TEXT    NOT NULL,
    paired_at   INTEGER NOT NULL
) STRICT;
