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
| `ha-server`   | Binaire qui assemble le tout et lit la configuration                        |

Briques principales : `tokio` (asynchrone), `axum` (web), `sqlx` + SQLite, `serde` (config/JSON),
`rumqttc` (MQTT), `tracing` (logs), `argon2` (mots de passe).

## Étapes

- [ ] **0. Fondations** : installer Rust, workspace Cargo, CI GitHub Actions (fmt, clippy, tests, audit des dépendances), Dependabot
- [ ] **1. Noyau** : bus d'événements + machine à états + entités de démo, en ligne de commande
- [ ] **2. Stockage** : enregistrer les entités et l'historique dans SQLite
- [ ] **3. Authentification + API** : comptes, jetons, API REST et WebSocket en local
- [ ] **4. MQTT** : piloter de vrais appareils (découverte automatique façon Home Assistant)
- [ ] **5. Automatisations** : « quand X, si Y, alors Z » en YAML
- [ ] **6. Interface web** : tableau de bord simple
- [ ] **7. Déploiement** : image Docker, accès à distance via VPN (jamais de port ouvert sur Internet)

Les règles de sécurité qui s'appliquent à chaque étape sont dans [SECURITY.md](SECURITY.md).
