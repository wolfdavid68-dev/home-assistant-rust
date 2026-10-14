-- Historique en graphiques (étape 10) : lire rapidement l'historique d'une
-- entité sur une période (« les dernières 24 heures »).
CREATE INDEX idx_state_history_entity_time ON state_history (entity_id, recorded_at);
