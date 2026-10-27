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

Toute la documentation, rangée par sujet : **[docs/README.md](docs/README.md)**. Pour commencer :

- [Le tableau de bord](docs/etape-6-interface-web.md)
- [Les automatisations](docs/etape-5-automatisations.md) et leur [éditeur](docs/editeur-automatisations.md)
- [Les appareils MQTT](docs/etape-4-mqtt.md), [Philips Hue](docs/hue.md) et [Somfy TaHoma](docs/somfy.md)
- [La tablette murale](docs/tablette-murale.md) et [la voix](docs/voix.md)
- [Le déploiement (Docker, VPN, HTTPS)](docs/etape-7-deploiement.md)

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
