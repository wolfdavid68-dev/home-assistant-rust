-- Notifications sur le téléphone, par le navigateur (Web Push, étape 50).

-- La paire de clés « VAPID » du serveur : elle signe chaque envoi, et le
-- service de notifications du navigateur (Google, Mozilla, Apple…) refuse
-- tout envoi qui n'est pas signé par elle. Une seule ligne (CHECK).
CREATE TABLE push_vapid (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    -- Clé privée (PKCS#8), chiffrée (voir secrets.rs).
    private_key TEXT    NOT NULL,
    -- Clé publique (65 octets, base64url) : donnée aux navigateurs.
    public_key  TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
) STRICT;

-- Un abonnement par navigateur (un téléphone, une tablette…).
CREATE TABLE push_subscriptions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- L'adresse du service de notifications, propre à ce navigateur.
    endpoint     TEXT    NOT NULL UNIQUE,
    -- Les clés du navigateur pour chiffrer le contenu (base64url) ; « auth »
    -- est un secret, chiffré comme les autres.
    p256dh       TEXT    NOT NULL,
    auth         TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    -- Ce que l'appareil veut recevoir : 1 ordres à valider, 2 alertes du
    -- serveur, 4 messages des automatisations.
    categories   INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    last_success INTEGER,
    failures     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX idx_push_subscriptions_user ON push_subscriptions (user_id);
