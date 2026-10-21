-- Tablettes murales (étape 78) : une session qui reste ouverte tant que la
-- tablette sert (prolongée à chaque usage), qu'un administrateur voit dans
-- Mon compte et peut fermer à distance.
--
-- wall_label   : le nom donné à la tablette (« Couloir ») ; NULL pour une
--                session ordinaire.
-- last_used_at : dernière prolongation (au plus une par jour), en
--                microsecondes depuis 1970.
ALTER TABLE access_tokens ADD COLUMN wall_label TEXT;
ALTER TABLE access_tokens ADD COLUMN last_used_at INTEGER;
