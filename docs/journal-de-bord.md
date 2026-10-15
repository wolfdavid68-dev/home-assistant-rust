# Étape 21 : le journal de bord, expliqué

## Le but

La lumière du salon s'est éteinte toute seule hier soir. Pourquoi ? Une automatisation ? Quelqu'un
de la famille ? Alexa ? L'historique (étape 10) montre **quand** elle s'est éteinte, pas
**pourquoi**. Comme dans Home Assistant, le **journal de bord** raconte ce qui s'est passé, en
phrases, avec la cause :

```text
Aujourd'hui
22:30  Bonne nuit            automatisation lancée     déclencheur : il est 22:30
22:30  Lumière du salon      s'est éteint              par l'automatisation « Bonne nuit »
22:41  Porte d'entrée        s'est ouvert
22:42  Lumière de l'entrée   s'est allumé              par David
23:05  Volet du salon        s'est fermé               à la voix (Alexa)
23:30  Lessive               terminé
```

## L'utiliser

Bouton **« Journal de bord »**, en haut du tableau de bord :

- **24 heures** ou **7 jours** ;
- un **appareil** en particulier, ou tous ;
- il se met à jour tout seul tant qu'il est ouvert.

Depuis l'historique d'un appareil (le petit graphique sur sa tuile), **« Voir dans le journal de
bord »** l'ouvre directement sur cet appareil.

Rien à configurer : il est gardé aussi longtemps que l'historique (`HA_HISTORY_DAYS`, 10 jours par
défaut), et il est dans les sauvegardes.

## Ce qui est noté, et ce qui ne l'est pas

**Noté** : lumières et prises, détecteurs (porte, mouvement, fumée…), volets, présence des
personnes et zone Maison, chauffage (marche / arrêt), soleil, entrées (étape 20), minuteurs,
scènes, scripts, notifications, automatisations lancées, démarrage du serveur, appareils
indisponibles.

**Pas noté**, pour que le journal reste lisible :

- les **mesures** (température, humidité, météo…) : elles changent sans cesse, c'est le rôle de
  l'historique et de ses courbes ;
- les changements d'**attributs** seuls (luminosité d'une lumière déjà allumée…) ;
- la **création** des appareils au démarrage.

## D'où vient la cause

Chaque commande porte maintenant son **origine** : la personne connectée au tableau de bord,
l'automatisation, le script ou la scène qui l'envoie, Alexa, ou le serveur lui-même (thermostat).
Quand un appareil change, le journal regarde la **dernière commande qu'il a reçue** :

- si elle date de **moins de 10 secondes**, c'est la cause ;
- une commande n'explique qu'**un seul** changement : si David allume une lumière et qu'elle
  s'éteint toute seule 5 secondes plus tard, la deuxième ligne n'aura pas de cause ;
- un appareil qui change tout seul (détecteur, interrupteur mural), ou un minuteur qui arrive à zéro,
  n'a pas de cause ;
- une commande du serveur lui-même (le thermostat qui allume le radiateur) n'affiche pas de cause.

C'est plus simple que Home Assistant (qui suit un « contexte » de bout en bout) mais suffisant pour
répondre à la question de tous les jours : « qui a fait ça ? ».

## Sécurité

- Le journal ne contient **aucun secret** : des noms d'appareils, des états, des noms
  d'utilisateur.
- Les textes sont **nettoyés** avant d'être rangés (caractères de contrôle remplacés, 200 caractères
  au plus), et toujours écrits par paramètres SQL. Un nom d'appareil piégé venu d'un appareil MQTT
  ne peut ni casser la base ni l'affichage (le tableau de bord n'utilise jamais de HTML).
- Il faut être connecté pour le lire, comme tout le reste de l'API (`GET /api/logbook`).

Le **journal d'activité** (en bas de la page) reste à part : c'est le journal d'audit,
signé, des actions sensibles (connexions, modifications). Le journal de bord, lui, raconte la vie de
la maison.

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [ha-core/src/origin.rs](../crates/ha-core/src/origin.rs) | `Origin` : qui envoie une commande |
| [ha-logbook/src/describe.rs](../crates/ha-logbook/src/describe.rs) | Les phrases (« s'est ouvert », « est à la maison »…) |
| [ha-logbook/src/lib.rs](../crates/ha-logbook/src/lib.rs) | La tâche : causes, écriture |
| [ha-storage/src/logbook.rs](../crates/ha-storage/src/logbook.rs) | La table `logbook` : écriture, lecture, purge |
| [engine.rs](../crates/ha-automation/src/engine.rs) | L'événement `automation_triggered` (nom et raison) |
| [app.js](../crates/ha-api/web/app.js) | La fenêtre du journal |

**Concepts Rust rencontrés :**

- **Ajouter un champ à une variante d'`enum`** (`origin` dans `Event::CallService`) : chaque
  `match` qui listait tous les champs a dû être revu. Ceux qui finissaient par `..` (« et le reste »)
  n'ont pas bougé : c'est l'intérêt de `..` quand on ne lit que certains champs.
- **`HashMap::remove`** pour « prendre » la cause : elle est rendue **et** oubliée en une seule
  opération, donc impossible de l'utiliser deux fois par erreur.
- **Une fonction pure** (`line_for`) qui reçoit l'heure en paramètre : les tests vérifient « 11
  secondes plus tard, plus de cause » sans attendre.
- **`Option` et `?`** : `data.get("entity_id")?.as_str()?.parse().ok()?` : la moindre valeur absente
  ou invalide, et la fonction renvoie `None`, sans `if` imbriqués.

## Testé

- Les phrases : chaque sorte d'appareil, et ce qui est laissé de côté (mesures, météo, attributs,
  création, `unknown`).
- Les causes : sans commande, rien ; « par david » dans les 10 secondes, une seule fois ; une
  commande plus récente remplace la précédente ; rien après 10 secondes ; rien pour le serveur ; un
  minuteur démarré « par david » mais terminé sans cause.
- La base : filtre par appareil, période, limite, ordre, purge ; un nom piégé
  (`'); DROP TABLE logbook; --` et un retour à la ligne) rangé tel quel, nettoyé.
- L'API : jeton obligatoire, filtres, refus des paramètres invalides.
- **Avec le vrai moteur** : la porte s'ouvre → « automatisation lancée (déclencheur : Porte d'entrée
  passe à « on ») » → « Lumière du salon s'est éteint (par l'automatisation …) », puis « s'est
  allumé (par david) ».
- **Sur un serveur en marche, dans Chromium** : entrées commandées depuis le tableau de bord, minuteur
  qui lance une automatisation à la fin, journal lu avec les bonnes causes ; filtre sur un appareil
  depuis son historique. Ce test a trouvé un vrai défaut, corrigé : un minuteur arrivé à zéro était
  noté « par david » (démarré par lui 8 secondes plus tôt). Aucune erreur, pas de débordement sur
  téléphone (les boutons du haut passent maintenant sur deux lignes).
