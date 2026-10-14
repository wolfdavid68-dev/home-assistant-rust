# Sécurité

Ce logiciel contrôle des appareils physiques (serrures, chauffage, prises…). Une faille peut avoir
des conséquences réelles, donc ces règles ne sont pas optionnelles.

## Code

- `#![forbid(unsafe_code)]` dans toutes les crates : pas de code `unsafe`.
- Pas de `unwrap()` / `expect()` sur des données venant de l'extérieur (réseau, fichiers, appareils).
- Toutes les entrées sont désérialisées dans des types stricts (`serde`, `deny_unknown_fields`) et validées.
- Requêtes SQL uniquement paramétrées (`sqlx`), jamais de concaténation de chaînes.
- Les logs ne contiennent jamais de mot de passe, de jeton ou de secret.

## Authentification

- Mots de passe hachés avec **Argon2id**, jamais stockés en clair.
- Jetons d'accès aléatoires (256 bits, générateur cryptographique), stockés **hachés**, comparés en temps constant.
- Limitation des tentatives de connexion (blocage temporaire après plusieurs échecs).
- Aucune route de l'API accessible sans authentification, sauf la page de connexion.

## Réseau

- Par défaut, le serveur écoute seulement en local (`127.0.0.1`). L'ouverture au réseau local est un choix explicite.
- **Ne jamais exposer le serveur directement sur Internet.** Accès à distance uniquement via VPN (WireGuard, Tailscale).
- MQTT : broker avec identifiants, et TLS dès qu'il sort de la machine.
  **Exception temporaire** : TLS pas encore disponible (la partie TLS de `rumqttc` 0.25 dépend de crates
  vulnérables, refusées par `cargo deny`). D'ici là, broker sur la même machine ou sur le réseau local
  uniquement, jamais joignable depuis Internet.

- Pont Philips Hue : HTTPS avec certificat **épinglé** à l'association (confiance au premier usage),
  jamais de vérification TLS désactivée.

## Secrets

- Aucun secret dans le dépôt Git. Ils vont dans `secrets.yaml` ou `.env`, ignorés par Git.
- Si un secret est poussé par erreur, on le **change** immédiatement (le supprimer de Git ne suffit pas).

## Dépendances

- Le moins de dépendances possible, et des crates connues et maintenues.
- `cargo audit` et `cargo deny` dans la CI : la CI échoue si une dépendance a une faille connue.
- Dependabot activé pour les mises à jour.
- `Cargo.lock` versionné.

## Processus

- Tout changement passe par une pull request ; la CI (fmt, clippy, tests, audit) doit être verte avant de fusionner.
- En Docker : utilisateur non-root, système de fichiers en lecture seule autant que possible.

## Signaler une faille

Dépôt privé : ouvrir une issue avec l'étiquette `security`.
