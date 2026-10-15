# Feuille de route

## Ce qu'on construit (et ce qu'on ne construit pas)

Home Assistant, c'est plus de 10 ans de travail, des milliers de contributeurs et plus de 2 000 intégrations.
Le réécrire entièrement n'est pas réaliste. L'objectif est donc un **noyau domotique en Rust**,
inspiré de Home Assistant, qui pilote **tes** appareils, puis qu'on enrichit petit à petit.

Chaque étape doit donner quelque chose qui tourne et qui est testé avant de passer à la suivante.

## Architecture visée

Un workspace Cargo découpé en crates :

| Crate         | Rôle                                                                       |
|---------------|----------------------------------------------------------------------------|
| `ha-core`     | Bus d'événements, machine à états (état de chaque entité), registre des services |
| `ha-storage`  | Persistance SQLite (registre des entités, historique)                      |
| `ha-auth`     | Utilisateurs, mots de passe (Argon2id), jetons d'accès                      |
| `ha-api`      | API HTTP + WebSocket (axum), authentifiée                                   |
| `ha-automation` | Automatisations en YAML : déclencheurs, conditions, actions              |
| `ha-mqtt`     | Première intégration : MQTT (Zigbee2MQTT, Tasmota, Shelly…)                 |
| `ha-hue`      | Philips Hue : lampes via le pont Hue (API locale v2)                        |
| `ha-somfy`    | Somfy TaHoma : volets, capteurs, alarme, chauffage (API locale)             |
| `ha-freebox`  | Freebox : présence, connexion Internet, nouveaux appareils (lecture seule) |
| `ha-areas`    | Pièces de la maison et noms lisibles (fichier `areas.yaml`)                 |
| `ha-notify`   | Notifications sur le téléphone avec ntfy                                    |
| `ha-sun`      | Soleil : lever, coucher, hauteur, calculés sans réseau                      |
| `ha-tls`      | Connexions HTTPS partagées (certificat épinglé ou vérifié)                  |
| `ha-server`   | Binaire qui assemble le tout et lit la configuration                        |

Briques principales : `tokio` (asynchrone), `axum` (web), `sqlx` + SQLite, `serde` (config/JSON),
`rumqttc` (MQTT), `tracing` (logs), `argon2` (mots de passe).

## Étapes

- [x] **0. Fondations** : installer Rust, workspace Cargo, CI GitHub Actions (fmt, clippy, tests, audit des dépendances), Dependabot
- [x] **1. Noyau** : bus d'événements + machine à états + entités de démo, en ligne de commande
- [x] **2. Stockage** : enregistrer les entités et l'historique dans SQLite ([guide](docs/etape-2-stockage.md))
- [x] **3. Authentification + API** : comptes, jetons, API REST et WebSocket en local ([guide](docs/etape-3-auth-api.md))
- [x] **4. MQTT** : piloter de vrais appareils (découverte automatique façon Home Assistant) ([guide](docs/etape-4-mqtt.md)) — sans TLS pour l'instant
- [x] **4 bis. Philips Hue** : lampes via le pont Hue, en local, certificat épinglé ([guide](docs/hue.md))
- [x] **4 ter. Somfy TaHoma** : volets, capteurs, alarme et chauffage en lecture, API locale ([guide](docs/somfy.md)) — commande du chauffage et de l'alarme à venir
- [x] **4 quater. Freebox** : présence à la maison, connexion Internet, alerte nouvel appareil, VPN WireGuard ([guide](docs/freebox.md))
- [x] **5. Automatisations** : « quand X, si Y, alors Z » en YAML ([guide](docs/etape-5-automatisations.md))
- [x] **6. Interface web** : tableau de bord en direct, ordinateur et téléphone ([guide](docs/etape-6-interface-web.md))
- [x] **7. Déploiement** : image Docker durcie, accès à distance via VPN, HTTPS avec Tailscale (jamais de port ouvert sur Internet) ([guide](docs/etape-7-deploiement.md))

### Deuxième partie : se rapprocher de Home Assistant

Les fonctions de Home Assistant les plus utiles au quotidien, une par une, dans cet ordre :

- [x] **8. Soleil** : lever, coucher, aube, crépuscule ; déclencheur et condition `sun` comme dans Home Assistant ([guide](docs/soleil.md))
- [x] **9. Pièces** : ranger les appareils par pièce (salon, chambre…) et leur donner un nom lisible ([guide](docs/pieces.md))
- [x] **10. Historique en graphiques** : courbes de température, frise des ouvertures, sur 24 h ou 7 jours ([guide](docs/historique.md))
- [x] **11. Scènes et scripts** : « soirée cinéma » en un bouton, suites d'actions réutilisables ([guide](docs/scenes-scripts.md))
- [x] **12. Notifications** : être prévenu sur son téléphone (alarme, fumée, nouvel appareil sur le Wi-Fi), avec ntfy ([guide](docs/notifications.md))
- [x] **13. Éditeur d'automatisations** : créer et modifier les automatisations depuis le tableau de bord ([guide](docs/editeur-automatisations.md))
- [x] **14. Alexa en local** : piloter volets et prises à la voix, sans cloud ni port ouvert ([guide](docs/alexa.md)) — pas encore essayé avec un vrai Echo

### Troisième partie : aller plus loin

- [x] **15. Météo locale** : conditions, température, prévisions sur 5 jours avec Open-Meteo (sans compte), capteurs pour les automatisations ([guide](docs/meteo.md))
- [x] **16. Thermostat** : un radiateur sur prise connectée piloté d'après un thermomètre, consigne, préréglages, fenêtre ouverte, programmation par automatisations ([guide](docs/thermostat.md))
- [x] **17. Automatisations plus riches** : conditions « ou » / « non », identifiants de déclencheurs, actions `if` / `choose` / `condition`, modes `restart` / `queued` / `parallel` ([guide](docs/automatisations-avancees.md))
- [x] **18. Sauvegardes automatiques** : chaque nuit, sans arrêter le serveur, vérifiées, 7 gardées, restauration en une commande ([guide](docs/sauvegardes.md))
- [x] **19. Personnes et zone Maison** : une personne regroupe ses appareils ; `zone.home` compte qui est là (« quand le dernier part ») ([guide](docs/personnes.md))
- [x] **20. Entrées** : interrupteurs, curseurs, listes, boutons et minuteurs virtuels (« mode vacances », « lessive finie »), déclencheur `event` ([guide](docs/entrees.md))
- [x] **21. Journal de bord** : ce qui s'est passé, en phrases, et pourquoi (« éteinte par l'automatisation Bonne nuit », « par David », « à la voix ») ([guide](docs/journal-de-bord.md))
- [x] **22. Automatisations encore plus riches** : déclencheur `time_pattern` (« toutes les 15 minutes »), actions `repeat` et `wait_for_trigger` (« attendre que la porte se ferme, 10 min au plus ») ([guide](docs/automatisations-encore.md))
- [x] **23. Comptes avec rôles** : administrateur, famille (tout piloter, pas modifier les automatisations), invité (lumières, prises, volets ; pas la présence, ni le garage) ([guide](docs/roles.md))
- [x] **24. Statistiques long terme** : un résumé par heure de chaque capteur (moyenne, min, max, consommation des compteurs), gardé des années ; courbes sur 30 jours et 1 an ([guide](docs/statistiques.md))
- [x] **25. Nouvelle mise en forme** : « Tout est calme, sauf… », plan des pièces, vue par pièce, cadran du thermostat, barre du bas sur téléphone ; thème clair le jour, sombre le soir ; polices embarquées, rien chargé depuis Internet ([guide](docs/mise-en-forme.md))
- [x] **26. Finitions** : accords (« porte ouverte »), boutons cachés aux invités quand ils ne servent à rien, mode tablette murale (écran allumé, plein écran, retour à l'accueil) ([guide](docs/mise-en-forme.md#finitions))
- [x] **27. Nouveaux appareils MQTT** : serrures (déverrouiller redemande le mot de passe), sirènes, vannes, aspirateurs, humidificateurs, chauffe-eau ([guide](docs/appareils-mqtt.md))
- [x] **28. Agenda et listes** : liste de courses, tâches et agenda de la maison gardés chez toi ; déclencheur `calendar` (« 30 minutes avant un rendez-vous ») ([guide](docs/agenda.md))
- [x] **29. MQTT complet** : volets, ventilateurs, thermostats, boutons, nombres, listes, textes, télécommandes (événements) et alarmes (désarmer redemande le mot de passe), comme Home Assistant ([guide](docs/appareils-mqtt.md#étape-29--volets-ventilateurs-thermostats-boutons-réglages-télécommandes-et-alarmes))
- [x] **30. Assistants du quotidien** : compteur, texte, date et heure (réveil réglable comme déclencheur) ; groupes, plages horaires, moyenne / min / max, seuil avec hystérésis, consommation par période ([guide](docs/assistants.md))
- [x] **31. Historique des exécutions et notifications** : pour chaque automatisation, ses 10 dernières exécutions pas à pas (« pourquoi la lumière ne s'est pas allumée ? ») ; notifications sur le tableau de bord ; alertes de réparation (appareil injoignable, entité inconnue, boucle) ([guide](docs/historique-executions.md))

En parallèle, dès que tu me donnes les informations : commande du chauffage Somfy (résultat de
`somfy devices`), puis de l'alarme (avec confirmation et journal).

Les règles de sécurité qui s'appliquent à chaque étape sont dans [SECURITY.md](SECURITY.md).
