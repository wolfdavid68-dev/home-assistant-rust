# Étape 24 : les statistiques long terme, expliqué

## Le but

L'historique détaillé (étape 10) garde **chaque** mesure, mais seulement 10 jours : au-delà, la base
grossirait sans fin (un capteur qui mesure chaque seconde, c'est 86 400 lignes par jour). Or les
vraies questions sont sur le long terme : « fait-il plus froid dans la chambre que l'hiver dernier ? »,
« combien d'électricité ce mois-ci ? ».

Comme Home Assistant, le serveur fait maintenant, **chaque heure**, un résumé de chaque capteur, gardé
**des années** :

- la **moyenne**, le **minimum** et le **maximum** de l'heure ;
- pour un **compteur** (électricité, gaz, eau), la **consommation** de l'heure.

## L'utiliser

Rien à configurer. Dans la fenêtre d'historique d'un capteur (le petit graphique sur sa tuile), deux
nouvelles périodes :

- **30 jours** et **1 an** : un point par jour, avec la bande min–max ;
- pour un compteur : des **barres**, la consommation de chaque jour, avec le total et la moyenne par
  jour.

« 24 heures » et « 7 jours » restent sur l'historique détaillé.

Les statistiques commencent à s'accumuler dès cette version : la vue « 1 an » se remplit au fil des
mois. Au premier démarrage, le serveur **rattrape** tout ce que l'historique détaillé contient encore
(les 10 derniers jours).

## Quels capteurs

- **Résumés** : les capteurs qui ont une **unité** (`°C`, `%`, `kWh`, `W`, `hPa`…).
- **Compteurs** : ceux qui ont `state_class: total` ou `total_increasing`, ou `device_class` `energy`,
  `gas` ou `water` (c'est ce qu'envoient les compteurs MQTT et le futur Linky).
- **Pas résumés** : interrupteurs, détecteurs, textes (leur historique sert à la frise, pas à une
  moyenne).

## Ce que veulent dire les chiffres

- **Moyenne pondérée par la durée** : 20 °C pendant 50 minutes puis 26 °C pendant 10 minutes, c'est
  21 °C en moyenne, pas 23. Un capteur qui envoie plus souvent quand il fait chaud ne fausse pas la
  moyenne.
- **Indisponible** (pile vide, Wi-Fi coupé) : ces moments sont **laissés de côté**, ils ne comptent
  pas comme des zéros.
- **Compteur remis à zéro** (nouveau compteur, coupure) : la consommation reste juste. 100 → 103 →
  remise à zéro → 1 → 2, c'est 3 + 1 + 1 = 5 consommés, pas −98.
- **Un jour, un mois** : en heure **locale** (minuit à Paris, pas minuit à Londres), changements
  d'heure compris.

## Place sur le disque

Une ligne par capteur et par heure : 100 capteurs, c'est environ 50 Mo par an. Les statistiques sont
dans la base, donc dans les sauvegardes (étape 18).

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [0013_statistics.sql](../crates/ha-storage/migrations/0013_statistics.sql) | Les tables `statistics` et `statistics_runs` |
| [statistics.rs](../crates/ha-storage/src/statistics.rs) | Le résumé d'une heure, le rattrapage, la lecture par jour ou par mois, la tâche horaire |
| [rest.rs](../crates/ha-api/src/rest.rs) | `GET /api/statistics/{entité}?period=day&days=365` |
| [app.js](../crates/ha-api/web/app.js) | Les vues 30 jours et 1 an, les barres de consommation |

Chaque heure (à l'heure pile, plus 15 secondes pour que l'historique ait fini d'écrire), la tâche
calcule l'heure qui vient de finir. Elle note chaque heure faite dans `statistics_runs` : après un
arrêt, une coupure ou une mise en veille, elle rattrape exactement ce qui manque, et ne refait jamais
une heure déjà faite.

**Un vrai défaut trouvé en essayant** : le premier essai faisait tout dans une seule longue
transaction (lire l'historique, puis écrire les résumés). Pendant ce temps, l'historique continuait
d'écrire, et SQLite a refusé (« database is locked »). Maintenant, les lectures se font d'abord, sans
rien bloquer, puis l'écriture dans une transaction courte.

**Concepts Rust rencontrés :**

- **Une fonction pure pour le cœur du calcul** (`summarize`) : elle reçoit l'heure, la valeur de
  départ et les changements, et rend le résumé. Aucune base, aucune horloge : les cas difficiles
  (remise à zéro, indisponible, valeur remplacée pile au début de l'heure) se testent en une ligne.
- **Une fermeture (`closure`) qui modifie des variables** : `close` ajoute la durée écoulée à la
  moyenne. Rust interdit de lire ces variables tant qu'elle peut encore les modifier ; il voit tout
  seul qu'après son dernier appel, c'est fini (les « durées de vie non lexicales »).
- **`div_euclid`** : arrondir à l'heure **en dessous**, même pour une date avant 1970 (où `/`
  arrondirait vers zéro, donc au-dessus).
- **`f64::min` comme fonction** : `fold(f64::INFINITY, f64::min)` trouve le plus petit d'une liste.

## Testé

- Le calcul : moyenne pondérée (21 °C, pas 23), moments indisponibles laissés de côté, valeur apparue
  en cours d'heure, compteur remis à zéro (5 consommés), compteur immobile (0, pas « inconnu »),
  valeurs illisibles refusées.
- La base, avec des mesures placées à des heures précises : deux heures calculées, une valeur
  remplacée **pile au début** de l'heure qui ne compte pas dans le minimum (défaut trouvé par ce
  test, corrigé), un deuxième rattrapage qui ne refait rien, le regroupement par **jour de Paris**
  (22 h UTC est déjà le lendemain), la consommation du mois, un capteur sans unité ignoré.
- L'API : jeton obligatoire, refus des périodes et durées invalides, interdit aux invités.
- **En vrai**, avec 40 jours d'historique générés (une température qui monte et descend chaque jour,
  un compteur électrique remis à zéro en route) : 1 439 heures rattrapées en quelques secondes au
  démarrage, sans erreur ; dans Chromium, les courbes 30 jours et 1 an, les barres de consommation
  (288 kWh, 9,3 kWh par jour) ; aucune erreur, pas de débordement sur téléphone.
