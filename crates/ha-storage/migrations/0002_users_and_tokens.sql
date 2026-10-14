-- Comptes utilisateurs et jetons d'accès (étape 3).
-- Les dates sont en microsecondes depuis 1970, comme dans 0001.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT    NOT NULL UNIQUE,
    -- Empreinte Argon2id (format PHC, contient le sel et les paramètres).
    -- Le mot de passe lui-même n'est jamais stocké.
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE access_tokens (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- SHA-256 du jeton. Le jeton lui-même n'est jamais stocké : une copie de
    -- la base ne permet pas de se connecter.
    token_hash BLOB    NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_access_tokens_user ON access_tokens (user_id);
CREATE INDEX idx_access_tokens_expires_at ON access_tokens (expires_at);
