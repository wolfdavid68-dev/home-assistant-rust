-- L'index (entity_id, id) faisait double emploi avec (entity_id, recorded_at),
-- créé par 0008 : SQLite range déjà `id` au bout de chaque index, donc
-- (entity_id, recorded_at) trie aussi par (entity_id, recorded_at, id).
-- La seule requête qui triait par `id` trie maintenant par date, puis par `id`.
-- Un index de moins, c'est une écriture de moins à chaque changement d'état.
DROP INDEX IF EXISTS idx_state_history_entity;
