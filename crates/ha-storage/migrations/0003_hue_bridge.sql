-- Association avec un pont Philips Hue (étape Hue).
-- Une seule ligne au plus (id = 1).

CREATE TABLE hue_bridge (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    host        TEXT    NOT NULL,
    bridge_id   TEXT    NOT NULL,
    -- Empreinte SHA-256 du certificat du pont, mémorisée à l'association :
    -- on refusera tout autre certificat (« épinglage »).
    cert_sha256 BLOB    NOT NULL,
    -- Clé d'application remise par le pont. C'est un secret : elle permet de
    -- piloter toutes les lampes. La base est lisible par toi seul (droits 600).
    app_key     TEXT    NOT NULL,
    paired_at   INTEGER NOT NULL
) STRICT;
