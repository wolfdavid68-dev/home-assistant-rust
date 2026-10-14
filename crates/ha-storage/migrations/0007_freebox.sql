-- Intégration Freebox (lecture seule).

-- Association avec la Freebox. Une seule ligne au plus (id = 1).
CREATE TABLE freebox_box (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    host        TEXT    NOT NULL,
    device_name TEXT    NOT NULL,
    -- Empreinte du certificat, épinglée à l'association.
    cert_sha256 BLOB    NOT NULL,
    -- Jeton d'application remis par la Freebox, chiffré (voir secrets.rs).
    app_token   TEXT    NOT NULL,
    paired_at   INTEGER NOT NULL
) STRICT;

-- Appareils dont on suit la présence (téléphones…) : adresse MAC et nom de
-- l'entité (device_tracker.<name>).
CREATE TABLE freebox_tracked (
    mac      TEXT    PRIMARY KEY,
    name     TEXT    NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
) STRICT;

-- Appareils déjà vus sur le réseau : un appareil absent de cette liste est
-- « nouveau », et signalé.
CREATE TABLE freebox_known (
    mac        TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    first_seen INTEGER NOT NULL
) STRICT;
