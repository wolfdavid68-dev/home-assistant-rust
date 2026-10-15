-- Rôles des comptes : administrateur, famille ou invité (étape 23).
-- Les comptes existants deviennent administrateurs : rien ne change pour eux.
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin'
    CHECK (role IN ('admin', 'family', 'guest'));
