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
- [Freebox : accès à distance par VPN, et présence à la maison, expliqué](docs/freebox.md)
- [Étape 8 : le soleil (lever, coucher, volets au crépuscule), expliqué](docs/soleil.md)
- [Étape 5 : les automatisations, expliqué](docs/etape-5-automatisations.md)
- [Modernisation du code, expliquée](docs/modernisation.md)
- [Étape 6 : l'interface web, expliquée](docs/etape-6-interface-web.md)
- [Étape 7 : le déploiement (Docker, VPN, HTTPS), expliqué](docs/etape-7-deploiement.md)
- [Renforcement de la sécurité : jetons chiffrés, double authentification, journal d'audit, expliqués](docs/renforcement-securite.md)

## Lancer le serveur

```bash
cargo run -p ha-server -- user add <nom>   # une seule fois : crée un compte
cargo run -p ha-server                     # tableau de bord sur http://127.0.0.1:8123/
```

Ou avec Docker (installation durable, accès à distance par VPN) : voir le
[guide de l'étape 7](docs/etape-7-deploiement.md).

```bash
docker compose up -d --build
docker compose run --rm ha-server user add <nom>
```
