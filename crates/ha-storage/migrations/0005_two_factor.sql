-- Double authentification (TOTP).
-- `totp_secret` : secret partagé avec l'appli d'authentification, chiffré
-- (voir ha-storage/src/secrets.rs). NULL si la double authentification
-- n'est pas activée pour ce compte.
-- `totp_last_step` : dernier créneau de 30 s dont le code a été accepté. Un
-- code déjà utilisé est refusé, même s'il est encore dans les temps.
ALTER TABLE users ADD COLUMN totp_secret TEXT;
ALTER TABLE users ADD COLUMN totp_last_step INTEGER;
