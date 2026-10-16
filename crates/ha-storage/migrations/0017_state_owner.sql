-- L'intégration qui publie chaque entité (étape 45) : `hue`, `mqtt`… Après
-- un redémarrage, une intégration qui ne revient pas peut ainsi mettre ses
-- entités « indisponibles », même avant d'avoir republié quoi que ce soit.
ALTER TABLE states ADD COLUMN owner TEXT;
