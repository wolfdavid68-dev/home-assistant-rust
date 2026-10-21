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
- [x] **32. Compteur Linky et bilan énergétique** : lecture de la TIC (modes historique et standard, module USB ou réseau), index heures creuses / pleines / Tempo, puissance, production ; consommation par jour, mois et année avec le coût ([guide](docs/energie.md))
- [x] **33. Vigilance Météo-France** : le niveau du département (vert à rouge) aujourd'hui et demain, chaque phénomène et ses heures, littoral compris ; dans les automatisations (« prévenir en vigilance orange ») et sur l'accueil ([guide](docs/vigilance.md))
- [x] **34. Modèles et déclencheurs plus fins** : des valeurs dans les messages (`{{ states('sensor.x') }}`, `trigger.to_state`…), déclencheurs et conditions sur un attribut, listes (`to: [Orange, Rouge]`), `not_from` / `not_to` ([guide](docs/modeles.md))
- [x] **35. Schéma façon n8n** : les automatisations en blocs reliés sur un canevas libre (glisser, tirer des fils, zoomer, pincer sur téléphone) ; les exécutions passées affichées sur les blocs ([guide](docs/schema.md))
- [x] **36. Style « plan d'architecte »** : en option, le tableau de bord en plan (papier quadrillé, encre bleue, ou blanche sur bleu la nuit) ; le dessin de ta maison au centre, avec un fond de nuit, des calques selon l'état des appareils (garage ouvert, porche allumé) et des valeurs posées dessus ; repères, cartouche, humidité des pièces ([guide](docs/plan-architecte.md))
- [x] **37. Mise en page « tableau » et thème « Maquette »** : comme les tableaux isométriques de Home Assistant, les pièces en petites tuiles à gauche, la maquette au centre avec repères et boutons rapides (porche, garage…), les volets en lignes ; fond sombre dégradé ([guide](docs/maquette.md))
- [x] **38. Onglets** : Maison, Garage, Piscine… chacun avec son dessin, ses boutons rapides et les pièces qu'il montre ; l'onglet ouvert est retenu sur chaque écran ([guide](docs/onglets.md))
- [x] **39. Ambiances météo** : un fond par temps (soleil, éclaircies, nuageux, brouillard, vent, pluie, averses, orage, grêle, neige, pluie et neige), de jour et de nuit, choisi d'après le soleil et la météo, le plus proche s'il en manque un ; une ambiance imposée pour essayer ([guide](docs/ambiances.md))
- [x] **40. Courbes dans les onglets** : comme l'onglet Garage de domo-blog, des courbes (puissance du NVR, du NAS, du compteur, température…) dans la colonne de gauche, sur 6 h, 24 h ou 7 jours, d'une couleur au choix ; toucher ouvre l'historique ([guide](docs/courbes.md))
- [x] **41. Toucher la maquette** : des zones tracées sur le dessin (le porche, la porte du garage…) ; un toucher allume, éteint, ouvre ou ferme, un appui long ouvre l'historique ; invisibles au repos, un éclat au toucher ([guide](docs/zones.md))
- [x] **42. Une fenêtre par pièce** : toucher une pièce (sa tuile ou une zone de la maquette) ouvre ses mesures, sa température sur 24 h et tous ses appareils, avec « Éteindre les lumières » ; montée du bas sur téléphone, à tirer vers le bas pour fermer ([guide](docs/piece.md))
- [x] **43. Effets météo animés** : sur la maquette, la pluie, les averses, l'orage et ses éclairs, la grêle, la neige, le brouillard, le vent, les étoiles par nuit claire ; selon la météo locale, imposés ou désactivés, trois intensités ; image fixe si l'appareil demande moins de mouvements ([guide](docs/effets.md))
- [x] **44. Faire les images avec l'IA** : dans le réglage du dessin, la demande à copier pour chaque image (description de la maison, fond de jour en diorama ou en plan, nuit, les 20 ambiances, les calques des lumières et des portes, les corrections), avec l'image à joindre et le nom à donner ; la méthode de domo-blog ([guide](docs/images-ia.md))
- [x] **45. Supervision des intégrations** : une intégration qui plante est relancée toute seule (attente croissante) ; en panne plus d'une minute, ses appareils passent « indisponibles », puis retrouvent leur valeur ; une seule alerte par intégration ; leur état dans « Mon compte » ([guide](docs/supervision.md))
- [x] **46. Des ordres avec réponse** : Hue et Somfy disent si l'ordre a abouti (refus et silence affichés) ; une intégration en panne refuse l'ordre d'office ; un ordre de plus de 5 s n'est jamais exécuté, même après une reconnexion ([guide](docs/ordres.md))
- [x] **47. La performance, mesurée** : Mon compte → Système (mémoire, processeur, temps de réponse, base) et un budget vérifié par la CI (image ≤ 80 Mo, mémoire ≤ 64 Mio, 95 % des réponses ≤ 25 ms) ; 40 Mio et 0,2 % de processeur sur la démo ([guide](docs/performance.md))
- [x] **48. Pièces protégées et heures calmes** : un ordre à la voix ou d'un invité sur une pièce protégée (la chambre, éventuellement la nuit seulement) attend qu'un membre de la famille le valide, depuis la cloche ; oublié après 10 minutes ; un clic de la famille est toujours obéi ([guide](docs/garde.md))
- [x] **49. L'écran de veille de la tablette murale** : après 5 minutes sans contact, la maison en grand, l'heure, la météo, ce qui demande de l'attention et les ordres à valider ; plus sombre la nuit, décalé chaque minute ; un contact pour revenir, sans rien commander ([guide](docs/veille.md))
- [x] **50. Les notifications du navigateur** : le téléphone sonne pour un ordre à valider, une alerte grave ou un message d'automatisation (`notify.appli`), même page fermée, sans appli à installer (Web Push, chiffré de bout en bout) ; chaque appareil choisit ce qu'il reçoit ([guide](docs/notifications-navigateur.md))
- [x] **51. Des boutons dans les notifications** : « Valider » et « Refuser » sur un ordre à valider, sans ouvrir le tableau de bord ; deux boutons au choix sur un message d'automatisation (événement `notification_action`) ; un ticket à usage unique par message et par appareil ([guide](docs/notifications-navigateur.md#étape-51--des-boutons-dans-la-notification))
- [x] **52. Les notifications se rangent toutes seules** : un ordre validé, refusé ou oublié remplace « Ordre à valider » sur tous les téléphones (« alice a validé : fermer Volet chambre ») ; une alerte grave réglée devient « Résolu » ; sans faire sonner ([guide](docs/notifications-navigateur.md#étape-52--les-notifications-se-rangent-toutes-seules))
- [x] **53. Des ordres vérifiés avant l'envoi** : position, consigne, luminosité, choix d'une liste… dans la plage que l'appareil annonce, sinon refusés tout de suite avec la raison (« consigne entre 7 et 25 (reçu 40) ») ; vaut aussi pour les automatisations, Alexa et la garde ([guide](docs/ordres.md#étape-53--vérifié-avant-lenvoi))
- [x] **54. Assist** : commander avec une phrase en français (« ferme le volet de la chambre », « mets le chauffage à 21 degrés », « qui est à la maison ? »), comprise sur le serveur sans cloud ; mêmes règles qu'un toucher (droits, garde, mot de passe, valeurs vérifiées) ([guide](docs/assist.md))
- [x] **55. Un assistant IA branché par MCP** : Claude (ou un autre) lit l'état de la maison et la pilote, comme le serveur MCP de Home Assistant et les agents de Moli OS ; ses ordres dans une pièce protégée attendent un humain, il ne touche jamais à une serrure, à l'alarme ni au garage, son jeton ne sert qu'à MCP, 60 appels par minute, tout est audité ([guide](docs/mcp.md))
- [x] **56. Jours ouvrés et jours fériés** : `binary_sensor.jour_ouvre`, `binary_sensor.demain_jour_ouvre` et `sensor.prochain_jour_ferie`, calculés sur place (Pâques comprise), avec le Vendredi saint et la Saint-Étienne en Alsace-Moselle, comme l'intégration Workday de Home Assistant ([guide](docs/jours-ouvres.md))
- [x] **57. Les agendas en ligne** : un fichier ICS (collecte des déchets, vacances scolaires, agenda Google partagé) devient un agenda en lecture seule, avec ses répétitions ; le déclencheur `calendar` marche avec lui (« la veille de la collecte »), comme « Remote Calendar » dans Home Assistant ([guide](docs/agendas-en-ligne.md))
- [x] **58. La simulation de présence** : maison vide, les lumières (et les volets) refont ce qu'elles ont fait il y a une semaine à la même heure ; armée par `switch.simulation_presence`, elle s'arrête au retour du premier ([guide](docs/simulation.md))
- [x] **59. Les statistiques d'historique** : combien de temps le chauffage a tourné aujourd'hui, combien de fois le garage s'est ouvert cette semaine, quelle part de la journée une lumière est restée allumée (`history_stats` de Home Assistant), calculés par la base ([guide](docs/assistants.md#les-statistiques-dhistorique-étape-59))
- [x] **60. Tempo, la couleur du jour et du lendemain** : lue sur le compteur Linky, sans Internet (`sensor.linky_tempo_aujourdhui`, `sensor.linky_tempo_demain`) ; tuile et panneau Énergie, un jour rouge signalé sur l'accueil ; dans les automatisations (« demain rouge : prévenir », « jour rouge : chauffage en éco ») ([guide](docs/energie.md#étape-60--la-couleur-tempo-du-jour-et-du-lendemain))
- [x] **61. Les piles faibles** : toutes les piles de la maison surveillées (capteurs Zigbee, détecteurs, serrures) ; alerte à 20 %, le téléphone sonne à 5 %, jusqu'à ce que la pile soit changée ; la liste dans Mon compte, la plus faible d'abord ([guide](docs/piles.md))
- [x] **62. Le délestage** : la maison approche de la puissance souscrite (lue sur le Linky) ? Les appareils les moins utiles sont coupés un par un avant que le disjoncteur ne saute, puis rallumés dans l'ordre inverse ; armé par `switch.delestage`, événement `delestage` ; la simulation de présence et le délestage cachés aux invités ([guide](docs/delestage.md))
- [x] **63. La rapidité des appareils, mesurée de bout en bout** : pour chaque ordre, le temps jusqu'au changement d'état de l'appareil, par intégration dans Mon compte → Système ; Somfy relevé toutes les 0,5 s juste après un ordre (un volet vu en 0,5 s au lieu de 2 s) ; alerte pour un appareil qui met plus de 3 s trois fois de suite ([guide](docs/rapidite.md))
- [x] **64. Des modèles qui calculent** : calculs, comparaisons, `and` / `or` / `not`, `x if … else y`, `in`, et `trigger.event.data` dans les modèles ; la condition `template` ; les capteurs modèles (`modeles.yaml`, comme `template:` de Home Assistant), recalculés dès que ce qu'ils lisent change ([guide](docs/calculs.md))
- [x] **65. L'intégrale et la dérivée** : une prise qui donne des watts a ses kWh (`integration`), utilisables dans le bilan énergétique avec leurs euros ; la vitesse de variation d'un capteur (`derivative`, « -6 °C/h » : une fenêtre ouverte), comme les assistants de Home Assistant ([guide](docs/assistants.md#lintégrale-et-la-dérivée-étape-65))
- [x] **66. Les Shelly récents, sans réglage** : prises et relais Shelly Gen2 et plus reconnus tout seuls par MQTT (marche/arrêt, puissance, énergie en kWh pour le bilan, tension), sous le nom donné dans l'appli Shelly ; en plus, les prises Hue s'affichent comme des prises ([guide](docs/shelly.md))
- [x] **67. La machine a fini** : lave-linge, lave-vaisselle, sèche-linge suivis d'après la puissance de leur prise (Arrêt, En marche, Terminé), sans confondre une pause de trempage avec la fin ; durée et énergie de chaque cycle ; événement `appareil_termine` pour la notification ([guide](docs/assistants.md#la-fin-du-lave-linge-étape-67))
- [x] **68. Le temps dans les modèles** : `now()` et ses champs (`now().hour`, `now().weekday()`…), `states.cover.garage.last_changed` et `as_timestamp(…)` pour compter des durées (« garage ouvert depuis 12 min ») ; un capteur modèle qui lit l'heure se recalcule chaque minute ([guide](docs/calculs.md#étape-68--le-temps-dans-les-modèles))
- [x] **69. Les rappels insistants** : le garage ouvert depuis 10 min, le congélateur trop chaud… un message, puis un rappel toutes les 30 min jusqu'à ce que ce soit réglé (« Garage fermé. ») ; « Compris » sur la notification ou la tuile fait taire, et le journal dit qui ; `rappels.yaml`, comme `alert` de Home Assistant ([guide](docs/rappels.md))
- [x] **70. La protection solaire des volets** : le soleil tape sur une façade, il fait chaud et il y a du soleil ? Ses volets descendent à la position choisie, puis remontent là où ils étaient quand le soleil est passé ; un volet bougé à la main est laissé tranquille 2 h ; armée par `switch.protection_solaire` ([guide](docs/protection-solaire.md))
- [x] **71. Les volets à mi-hauteur sur le dessin** : un calque « suit la position du volet » dessine le volet descendu de ce qu'il faut (à 25 %, aux trois quarts), son bord suivant la pente de la fenêtre, et le fait glisser quand il bouge ([guide](docs/plan-architecte.md#étape-71--les-volets-à-mi-hauteur))
- [x] **72. La découpe automatique des calques** : dans le réglage du dessin, deux images (tout éteint, puis allumé ou volets fermés) ; chaque endroit qui change devient un calque, à nommer et relier à une lumière ou à un volet, sans GIMP ni Photoshop pour la découpe ([guide](docs/images-ia.md#étape-72--la-découpe-automatique))
- [x] **73. Les enceintes Sonos et les annonces vocales** : chaque enceinte dans le tableau de bord (ce qu'elle joue, lecture, pause, volume), et la maison qui parle (« Le linge est fini ») avec une voix française qui tourne sur le PC (Piper) ; la musique reprend ensuite au même endroit et au même volume ; `tts.speak` dans les automatisations, sans compte ni cloud ([guide](docs/sonos.md))
- [x] **74. L'agenda TimeTree de la famille** : lu en lecture seule avec un compte dédié à la maison (mot de passe chiffré), affiché à côté de l'agenda de la maison, répétitions comprises ; `trigger.calendar_event.summary` dans les automatisations (« dans 30 minutes : dentiste », annoncé sur les enceintes) ([guide](docs/timetree.md))
- [x] **75. L'agenda façon Google Agenda** : un onglet Agenda (semaine, jour, mois, liste) qui réunit TimeTree, l'agenda de la maison, les vacances et la collecte, une couleur par agenda ou par étiquette TimeTree ; un rendez-vous s'ouvre avec son lieu, son étiquette et ses participants ; sur la page Maison, aujourd'hui, la semaine et le prochain rendez-vous ([guide](docs/agenda-semaine.md))
- [x] **76. Le tableau de bord plus léger** : les fichiers partent compressés (gzip) et ne sont plus renvoyés tant qu'ils n'ont pas changé (empreinte `ETag`, réponse `304`) : 162 Kio au lieu de 593 à la première ouverture, presque rien ensuite ; le gros `app.js` rangé en fichiers par thème ([guide](docs/tableau-leger.md))
- [x] **77. Le mini-PC dédié à la maison** : une machine sous Debian qui ne fait que la maison, comme Home Assistant OS ; `sudo ./installer.sh` installe le serveur en service systemd (démarre à l'allumage, se relance seul, protégé : note 1,8 de `systemd-analyze security`), la commande `sudo ha-maison …` pour l'administrer, une archive Linux jointe à chaque version, et le guide de A à Z (BIOS, Debian, adresse fixe, mises à jour, sauvegardes, voix) ([guide](docs/mini-pc.md))

En parallèle, dès que tu me donnes les informations : commande du chauffage Somfy (résultat de
`somfy devices`), puis de l'alarme (avec confirmation et journal).

Les règles de sécurité qui s'appliquent à chaque étape sont dans [SECURITY.md](SECURITY.md).
