# home-assistant-rust

Un noyau domotique en Rust, inspiré de Home Assistant.

![Tableau de bord](docs/images/tableau-de-bord-ordinateur.png)

- [Feuille de route](ROADMAP.md)
- [Règles de sécurité](SECURITY.md)

## Guides

- [Étape 1 : le noyau, expliqué](docs/etape-1-noyau.md)
- [Étape 2 : le stockage SQLite, expliqué](docs/etape-2-stockage.md)
- [Étape 3 : comptes, authentification et API, expliqués](docs/etape-3-auth-api.md)
- [Étape 4 : les vrais appareils avec MQTT, expliqué](docs/etape-4-mqtt.md)
- [Philips Hue : piloter tes lampes, expliqué](docs/hue.md)
- [Somfy TaHoma : volets, capteurs, alarme et chauffage, expliqué](docs/somfy.md)
- [Étape 5 : les automatisations, expliqué](docs/etape-5-automatisations.md)
- [Modernisation du code, expliquée](docs/modernisation.md)
- [Étape 6 : l'interface web, expliquée](docs/etape-6-interface-web.md)

## Lancer le serveur

```bash
cargo run -p ha-server -- user add <nom>   # une seule fois : crée un compte
cargo run -p ha-server                     # tableau de bord sur http://127.0.0.1:8123/
```
