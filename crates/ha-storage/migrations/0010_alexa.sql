-- Alexa en local (étape 14) : chaque appareil exposé reçoit un numéro de
-- « lampe Hue ». Alexa retient les appareils par ce numéro : il ne doit
-- jamais changer, même si l'ordre du fichier alexa.yaml change.
CREATE TABLE alexa_lights (
    entity_id TEXT    PRIMARY KEY,
    number    INTEGER NOT NULL UNIQUE
) STRICT;
