-- Journal d'audit : qui a fait quoi, quand, et d'où.
--
-- Chaque ligne est signée (HMAC-SHA256, avec une clé dérivée de ha.key) et
-- contient la signature de la ligne précédente : modifier ou supprimer une
-- ligne au milieu du journal se voit (« ha-server audit verify »).
CREATE TABLE audit_log (
    id       INTEGER PRIMARY KEY,
    at       INTEGER NOT NULL,
    -- Compte à l'origine de l'action, ou « ligne de commande ».
    actor    TEXT    NOT NULL,
    -- Action : login, login_failed, logout, service, state_set…
    action   TEXT    NOT NULL,
    -- Sur quoi : entité, compte…
    target   TEXT,
    detail   TEXT,
    -- D'où : adresse IP, ou « local ».
    source   TEXT    NOT NULL,
    prev_mac BLOB    NOT NULL,
    mac      BLOB    NOT NULL
) STRICT;

CREATE INDEX idx_audit_log_at ON audit_log (at);
