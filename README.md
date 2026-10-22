# home-assistant-rust

Un noyau domotique en Rust, inspiré de Home Assistant.

![Tableau de bord](docs/images/tableau-de-bord-ordinateur.png)

- **[Installer chez soi (Windows ou Linux)](docs/installation.md)** : le programme tout prêt est
  sur la [page des versions](https://github.com/wolfdavid68-dev/home-assistant-rust/releases) ;
  ou un [mini-PC dédié à la maison](docs/mini-pc.md), qui démarre tout seul
- [Feuille de route](ROADMAP.md)
- [Règles de sécurité](SECURITY.md)
- Licence [MIT](LICENSE) : libre d'utiliser, modifier et redistribuer, en gardant la mention de
  licence

## Guides

- [Étape 1 : le noyau, expliqué](docs/etape-1-noyau.md)
- [Étape 2 : le stockage SQLite, expliqué](docs/etape-2-stockage.md)
- [Étape 3 : comptes, authentification et API, expliqués](docs/etape-3-auth-api.md)
- [Étape 4 : les vrais appareils avec MQTT, expliqué](docs/etape-4-mqtt.md)
- [Philips Hue : piloter tes lampes, expliqué](docs/hue.md)
- [Somfy TaHoma : volets, capteurs, alarme et chauffage, expliqué](docs/somfy.md)
- [Freebox : accès à distance par VPN, et présence à la maison, expliqué](docs/freebox.md)
- [Étape 8 : le soleil (lever, coucher, volets au crépuscule), expliqué](docs/soleil.md)
- [Étape 9 : les pièces, expliqué](docs/pieces.md)
- [Étape 10 : l'historique en graphiques, expliqué](docs/historique.md)
- [Étape 11 : scènes et scripts, expliqué](docs/scenes-scripts.md)
- [Étape 12 : les notifications sur le téléphone, expliqué](docs/notifications.md)
- [Étape 13 : l'éditeur d'automatisations, expliqué](docs/editeur-automatisations.md)
- [Étape 14 : Alexa en local, expliqué](docs/alexa.md)
- [Étape 15 : la météo locale, expliquée](docs/meteo.md)
- [Étape 16 : le thermostat, expliqué](docs/thermostat.md)
- [Étape 17 : des automatisations plus riches, expliqué](docs/automatisations-avancees.md)
- [Étape 18 : les sauvegardes automatiques, expliqué](docs/sauvegardes.md)
- [Étape 19 : personnes et zone Maison, expliqué](docs/personnes.md)
- [Étape 20 : les entrées (interrupteurs, curseurs, minuteurs…), expliqué](docs/entrees.md)
- [Étape 21 : le journal de bord, expliqué](docs/journal-de-bord.md)
- [Étape 22 : répéter, attendre, toutes les N minutes, expliqué](docs/automatisations-encore.md)
- [Étape 23 : les comptes avec rôles, expliqué](docs/roles.md)
- [Étape 24 : les statistiques long terme, expliqué](docs/statistiques.md)
- [Étape 25 : la nouvelle mise en forme (thèmes clair et sombre), expliquée](docs/mise-en-forme.md)
- [Étape 27 : serrures, sirènes, vannes, aspirateurs, humidificateurs, chauffe-eau, expliqué](docs/appareils-mqtt.md)
- [Étape 28 : l'agenda et les listes de la maison, expliqué](docs/agenda.md)
- [Étape 29 : volets, ventilateurs, thermostats, boutons, télécommandes et alarmes MQTT, expliqué](docs/appareils-mqtt.md#étape-29--volets-ventilateurs-thermostats-boutons-réglages-télécommandes-et-alarmes)
- [Étape 30 : les assistants du quotidien (groupes, plages horaires, moyennes, seuils, consommation), expliqué](docs/assistants.md)
- [Étape 31 : l'historique des exécutions, les notifications et les alertes de réparation, expliqué](docs/historique-executions.md)
- [Étape 32 : le compteur Linky et le bilan énergétique, expliqué](docs/energie.md)
- [Étape 33 : la vigilance Météo-France, expliquée](docs/vigilance.md)
- [Étape 34 : les modèles et les déclencheurs plus fins, expliqués](docs/modeles.md)
- [Étape 35 : le schéma des automatisations façon n8n, expliqué](docs/schema.md)
- [Étape 36 : le style « plan d'architecte » (et le dessin de ta maison), expliqué](docs/plan-architecte.md)
- [Étape 37 : la mise en page « tableau » et le thème « Maquette », expliqués](docs/maquette.md)
- [Étape 38 : les onglets du tableau de bord (Maison, Garage, Piscine…), expliqués](docs/onglets.md)
- [Étape 39 : les ambiances météo du dessin (jour, nuit, pluie, neige…), expliquées](docs/ambiances.md)
- [Étape 40 : les courbes dans les onglets (puissance, température…), expliquées](docs/courbes.md)
- [Étape 41 : toucher la maquette pour commander la maison, expliqué](docs/zones.md)
- [Étape 42 : une fenêtre par pièce, expliquée](docs/piece.md)
- [Étape 43 : les effets météo animés sur la maquette, expliqués](docs/effets.md)
- [Étape 44 : faire les images de ta maison avec l'IA, expliqué](docs/images-ia.md)
- [Étape 45 : la supervision des intégrations, expliquée](docs/supervision.md)
- [Étape 46 : des ordres avec réponse, expliqués](docs/ordres.md)
- [Étape 47 : la performance, mesurée, expliquée](docs/performance.md)
- [Étape 48 : pièces protégées et heures calmes, expliquées](docs/garde.md)
- [Étape 49 : l'écran de veille de la tablette murale, expliqué](docs/veille.md)
- [Étape 50 : les notifications du navigateur, expliquées](docs/notifications-navigateur.md)
- [Étape 51 : des boutons dans les notifications](docs/notifications-navigateur.md#étape-51--des-boutons-dans-la-notification)
- [Étape 52 : les notifications se rangent toutes seules](docs/notifications-navigateur.md#étape-52--les-notifications-se-rangent-toutes-seules)
- [Étape 53 : des ordres vérifiés avant l'envoi](docs/ordres.md#étape-53--vérifié-avant-lenvoi)
- [Étape 54 : Assist, expliqué](docs/assist.md)
- [Étape 55 : brancher un assistant IA (MCP), expliqué](docs/mcp.md)
- [Étape 56 : jours ouvrés et jours fériés, expliqués](docs/jours-ouvres.md)
- [Étape 57 : les agendas en ligne, expliqués](docs/agendas-en-ligne.md)
- [Étape 58 : la simulation de présence, expliquée](docs/simulation.md)
- [Étape 59 : les statistiques d'historique](docs/assistants.md#les-statistiques-dhistorique-étape-59)
- [Étape 60 : la couleur Tempo du jour et du lendemain, expliquée](docs/energie.md#étape-60--la-couleur-tempo-du-jour-et-du-lendemain)
- [Étape 61 : les piles faibles, expliqué](docs/piles.md)
- [Étape 62 : le délestage, expliqué](docs/delestage.md)
- [Étape 63 : la rapidité des appareils, mesurée de bout en bout, expliqué](docs/rapidite.md)
- [Étape 64 : des modèles qui calculent, la condition template et les capteurs modèles, expliqué](docs/calculs.md)
- [Étape 65 : l'intégrale et la dérivée, expliquées](docs/assistants.md#lintégrale-et-la-dérivée-étape-65)
- [Étape 66 : les prises et relais Shelly récents, sans réglage, expliqué](docs/shelly.md)
- [Étape 67 : la fin du lave-linge, expliquée](docs/assistants.md#la-fin-du-lave-linge-étape-67)
- [Étape 68 : le temps dans les modèles, expliqué](docs/calculs.md#étape-68--le-temps-dans-les-modèles)
- [Étape 69 : les rappels insistants, expliqués](docs/rappels.md)
- [Étape 70 : la protection solaire des volets, expliquée](docs/protection-solaire.md)
- [Étape 71 : les volets à mi-hauteur sur le dessin](docs/plan-architecte.md#étape-71--les-volets-à-mi-hauteur)
- [Étape 72 : la découpe automatique des calques](docs/images-ia.md#étape-72--la-découpe-automatique)
- [Étape 73 : les enceintes Sonos et les annonces vocales, expliquées](docs/sonos.md)
- [Étape 74 : l'agenda TimeTree de la famille, expliqué](docs/timetree.md)
- [Étape 75 : l'agenda façon Google Agenda, expliqué](docs/agenda-semaine.md)
- [Étape 76 : le tableau de bord plus léger, expliqué](docs/tableau-leger.md)
- [Étape 77 : le mini-PC dédié à la maison, expliqué](docs/mini-pc.md)
- [Étape 78 : la tablette murale associée, expliquée](docs/tablette-murale.md)
- [Étape 79 : le HTTPS sur le réseau de la maison, expliqué](docs/https-maison.md)
- [Étape 80 : la vue hub de la tablette murale, expliquée](docs/hub.md)
- [Étape 81 : la liste de courses d'Iglou, expliquée](docs/iglou.md)
- [Étape 82 : la sonnette Ring, expliquée](docs/sonnette.md)
- [Étape 83 : toute la maison en une commande, expliquée](docs/tout-compris.md)
- [Étape 84 : parler à la maison depuis la tablette, expliqué](docs/voix.md)
- [Étape 85 : les automatisations toutes prêtes, expliquées](docs/pretes.md)
- [Étape 86 : la maison SmartThings de Samsung, expliquée](docs/smartthings.md)
- [Étape 87 : les colis Amazon sur la tablette, expliqués](docs/colis.md)
- [Étape 88 : la réponse vocale et la tablette plus rapides, expliquées](docs/plus-rapide.md)
- [Étape 89 : un tableau sans doublons et un écran de veille sans cadre, expliqués](docs/interface-allegee.md)
- [Étape 90 : un onglet par pièce, et tout sur un écran, expliqués](docs/onglets-pieces.md)
- [Étape 5 : les automatisations, expliqué](docs/etape-5-automatisations.md)
- [Modernisation du code, expliquée](docs/modernisation.md)
- [Étape 6 : l'interface web, expliquée](docs/etape-6-interface-web.md)
- [Étape 7 : le déploiement (Docker, VPN, HTTPS), expliqué](docs/etape-7-deploiement.md)
- [Renforcement de la sécurité : jetons chiffrés, double authentification, journal d'audit, expliqués](docs/renforcement-securite.md)

## Lancer le serveur

```bash
cargo run -p ha-server -- user add <nom>   # une seule fois : crée ton compte (administrateur)
cargo run -p ha-server                     # tableau de bord sur http://127.0.0.1:8123/
```

Ou avec Docker (installation durable, accès à distance par VPN) : voir le
[guide de l'étape 7](docs/etape-7-deploiement.md).

```bash
docker compose up -d --build
docker compose run --rm ha-server user add <nom>
```
