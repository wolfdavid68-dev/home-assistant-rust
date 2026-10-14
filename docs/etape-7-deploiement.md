# Étape 7 : le déploiement (Docker, VPN, HTTPS), expliqué

## Le but

Jusqu'ici, le serveur tournait avec `cargo run`, dans une fenêtre ouverte. Pour une vraie
installation, on veut :

- qu'il **tourne tout seul**, redémarre après une coupure, et garde ses données ;
- qu'il soit **enfermé** : même si une faille était trouvée, l'attaquant n'aurait presque rien à
  sa portée ;
- y accéder **depuis l'extérieur**, avec ton téléphone, **sans jamais ouvrir de port sur ta box
  Internet**.

```
   téléphone (4G, ailleurs)                       chez toi
  ┌──────────────────────┐   tunnel chiffré   ┌──────────────────────────────────────────┐
  │ Tailscale            │◄══════════════════►│ Tailscale ──► 127.0.0.1:8123 ──► conteneur│
  │ https://maison.….ts.net                   │                                ha-server │
  └──────────────────────┘                    │   (aucun port ouvert sur la box)         │
                                              └──────────────────────────────────────────┘
```

## Docker en trois mots

- Une **image** : le programme et tout ce dont il a besoin, empaquetés une fois pour toutes.
- Un **conteneur** : une image en train de tourner, isolée du reste de la machine.
- Un **volume** : un dossier géré par Docker, qui survit aux mises à jour. Ta base `ha.db` y est.

Notre image est construite en deux temps ([Dockerfile](../Dockerfile)) : une grosse image Rust
compile le programme, puis on ne garde que le programme, posé sur une image « distroless ». Elle
ne contient **ni shell, ni gestionnaire de paquets, ni curl** : 14 Mo en tout. Un attaquant qui
prendrait la main sur le programme ne trouverait aucun outil pour aller plus loin.

## Quelle machine ?

Le serveur doit rester allumé. Ton PC Windows convient pour essayer. Pour de bon, une petite
machine qui reste allumée est plus adaptée : mini-PC, Raspberry Pi 4 ou 5, ou NAS qui sait
faire tourner Docker (Synology…). Les commandes sont les mêmes partout.

Sous Windows, installe **Docker Desktop** (il utilise WSL 2, il te le proposera).

## Installer

Dans le dossier du projet :

```powershell
docker compose up -d --build
```

La première construction prend quelques minutes (compilation optimisée). Ensuite :

```powershell
docker compose ps                                  # état : « healthy » = tout va bien
docker compose run --rm ha-server user add david   # crée ton compte
```

Ouvre <http://127.0.0.1:8123> sur cette machine : c'est le tableau de bord.

**Associer Hue et Somfy** : mêmes commandes qu'avant, précédées de `docker compose run --rm`, puis
un redémarrage pour que le serveur les prenne en compte :

```powershell
docker compose run --rm ha-server hue pair 192.168.1.20
docker compose run --rm ha-server somfy pair 192.168.1.30
docker compose restart
```

**Automatisations** : mets ton fichier dans `config/automations.yaml` (le dossier `config` est
lu par le conteneur, en lecture seule), puis `docker compose restart`.

**MQTT** : décommente les lignes `HA_MQTT_…` et `secrets` dans [compose.yaml](../compose.yaml),
et mets le mot de passe du broker dans `secrets/mqtt_password` (ignoré par Git).

**Autres commandes utiles** :

```powershell
docker compose logs -f        # logs en direct (Ctrl+C pour quitter)
docker compose down           # arrêter (les données restent dans le volume)
git pull ; docker compose up -d --build   # mettre à jour
```

## Ouvrir au réseau local (facultatif)

Par défaut, le tableau de bord n'est joignable **que depuis la machine elle-même**. Pour y accéder
depuis ton téléphone sur ton Wi-Fi, crée un fichier `.env` à côté de `compose.yaml` :

```
HA_PUBLISH=192.168.1.10
```

(l'adresse de la machine sur ton réseau), puis `docker compose up -d`. Avec Tailscale (ci-dessous),
ce n'est même pas nécessaire : ton téléphone passe par le tunnel, chez toi comme ailleurs.

## Accès à distance : un VPN, jamais un port ouvert

Ouvrir le port 8123 sur ta box Internet mettrait ton tableau de bord face à tous les robots
d'Internet, qui essaient des mots de passe et des failles en permanence. **On ne le fait pas.**
À la place, un **VPN** : un tunnel chiffré que seuls tes appareils peuvent emprunter.

### Solution conseillée : Tailscale

[Tailscale](https://tailscale.com) relie tes appareils par des tunnels **WireGuard**, sans rien
ouvrir sur ta box. C'est gratuit pour un usage personnel.

1. Installe Tailscale sur la machine du serveur **et** sur ton téléphone, avec le même compte.
2. Dans la console d'administration Tailscale, rubrique DNS : active **MagicDNS** et
   **HTTPS Certificates**.
3. Sur la machine du serveur :

   ```powershell
   tailscale serve --bg 8123
   ```

4. Sur ton téléphone (Tailscale activé), ouvre l'adresse affichée, du genre
   `https://maison.ton-reseau.ts.net`. Ajoute-la à l'écran d'accueil : c'est une appli.

Ce qu'on y gagne :

- **HTTPS avec un vrai certificat** : connexion chiffrée de bout en bout, cadenas dans le
  navigateur, sans avertissement à ignorer.
- Le serveur reste en écoute sur `127.0.0.1` uniquement : rien n'est exposé, même sur ton réseau
  local. Seuls les appareils de **ton** réseau Tailscale peuvent l'atteindre.

Deux précautions :

- **N'utilise jamais `tailscale funnel`** : cette commande-là publie le service sur Internet,
  exactement ce qu'on veut éviter.
- Le nom du certificat (`maison.ton-reseau.ts.net`) devient public, comme tous les certificats
  HTTPS (journaux de transparence). Choisis un nom de machine neutre, sans ton nom ni ton adresse.

### Autre solution : le VPN de ta box Internet

Certaines box ont un serveur VPN **WireGuard** intégré (la Freebox, par exemple : voir le
[guide Freebox](freebox.md#1-laccès-à-distance-par-le-vpn-de-la-freebox) pas à pas). Tu l'actives
dans l'interface de la box, tu installes l'appli WireGuard sur ton téléphone, et une fois connecté
tu es « chez toi » : le tableau de bord s'ouvre à son adresse locale (avec `HA_PUBLISH`, voir
plus haut). C'est la box qui reçoit les connexions, et WireGuard ne répond à personne qui n'a pas
la bonne clé. En revanche, il n'y a pas de HTTPS : la connexion est chiffrée par le tunnel, mais
pas sur ton Wi-Fi local.

### Et pourquoi pas de HTTPS intégré au serveur ?

Pour du HTTPS sans avertissement, il faut un certificat signé pour un nom de domaine. Sur un
réseau local, le seul moyen simple serait un certificat « auto-signé ». Le navigateur afficherait
alors une alerte à chaque visite, et t'habituer à cliquer sur « continuer quand même » est
justement ce qui rend une vraie attaque possible un jour. `tailscale serve` fournit un vrai
certificat, renouvelé tout seul : c'est plus sûr que ce qu'on écrirait nous-mêmes.

## Sauvegarder et restaurer

Toutes tes données sont dans `ha.db` : comptes, historique, associations Hue et Somfy. Le petit
outil `sauvegarde` de [compose.yaml](../compose.yaml) la copie dans le dossier `sauvegardes/`
(ignoré par Git). On arrête le serveur pendant la copie, pour avoir une base cohérente.

**Sauvegarder** :

```powershell
docker compose stop ha-server
docker compose run --rm sauvegarde sh -c 'cp /data/ha.db /sauvegardes/ha-$(date +%F).db'
docker compose start ha-server
```

**Restaurer** (ici le fichier `sauvegardes/ha-2026-10-14.db`) :

```powershell
docker compose stop ha-server
docker compose run --rm sauvegarde sh -c 'cp /sauvegardes/ha-2026-10-14.db /data/ha.db && chown 65532:65532 /data/ha.db && rm -f /data/ha.db-wal /data/ha.db-shm'
docker compose start ha-server
```

Pourquoi pas un simple `docker cp` ? Le fichier copié appartiendrait à l'administrateur (root),
et le serveur, qui tourne volontairement sans droits, ne pourrait plus ouvrir sa base. Le
`chown 65532:65532` lui rend la propriété du fichier. (Les apostrophes simples `'…'` fonctionnent
telles quelles dans PowerShell comme sous Linux.)

Tes sauvegardes ne contiennent aucun mot de passe en clair (ils sont hachés avec Argon2id), et
les jetons du pont Hue et de la box Somfy y sont **chiffrés**. La clé qui les déchiffre n'est pas
dans `/data` mais dans un volume à part (`ha-cle`) : une sauvegarde volée ne permet pas de piloter
tes appareils.

**La clé, à mettre de côté une fois pour toutes** (dans ton gestionnaire de mots de passe) :

```powershell
docker compose run --rm sauvegarde cat /cle/ha.key
```

Si tu la perds (en réinstallant tout sur une autre machine, par exemple), rien de grave : le
serveur démarre quand même et te demande de refaire `hue pair` et `somfy pair`. Pour la remettre en
place :

```powershell
docker compose run --rm sauvegarde sh -c 'echo COLLE-ICI-LA-CLE > /cle/ha.key && chown 65532:65532 /cle/ha.key && chmod 600 /cle/ha.key'
```

## Les protections, une par une

Tout est dans [compose.yaml](../compose.yaml) :

| Réglage | Ce qu'il fait |
|---|---|
| `ports: "127.0.0.1:8123:8123"` | Joignable seulement depuis la machine, par défaut |
| `user: "65532:65532"` | Le programme tourne sans aucun droit d'administrateur |
| `read_only: true` | Le système de fichiers est en lecture seule (sauf `/data`, `/cle` et `/tmp`) |
| `cap_drop: ALL` | Retire toutes les « capacités » spéciales du noyau Linux |
| `no-new-privileges` | Interdit de regagner des droits, même par un programme spécial |
| `mem_limit`, `pids_limit` | Un emballement ne peut pas faire tomber toute la machine |
| `./config:/config:ro` | Les automatisations sont lues, jamais modifiables par le conteneur |
| Image distroless, épinglée par empreinte | Ni shell ni outils ; une image modifiée sur le registre serait refusée |
| `.dockerignore` | Ta base, tes secrets et `automations.yaml` ne partent jamais dans l'image |
| `logging: max-size` | Les logs ne remplissent pas le disque |
| Outil `sauvegarde` : `network_mode: none` | Il n'a aucun accès au réseau, et ne démarre qu'à la demande |

Une limite à connaître : la protection contre les essais de mots de passe compte les échecs
**par adresse IP**. Derrière Docker Desktop ou `tailscale serve`, les connexions peuvent toutes sembler
venir de la même adresse. Cinq échecs bloquent donc tout le monde pendant 15 minutes, et pas
seulement le fautif. C'est gênant, mais c'est du côté sûr : la protection contre les attaques
reste entière.

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [Dockerfile](../Dockerfile) | Construction en deux étapes, image finale distroless, contrôle de santé |
| [compose.yaml](../compose.yaml) | Lancement, volume, protections |
| [.dockerignore](../.dockerignore) | Ce qui n'est jamais envoyé à Docker |
| [health.rs](../crates/ha-server/src/health.rs) | `ha-server healthcheck` : le programme vérifie qu'il répond |
| [main.rs](../crates/ha-server/src/main.rs) | Arrêt propre sur `SIGTERM` |
| [ci.yml](../.github/workflows/ci.yml) | La CI construit l'image, la démarre et vérifie l'arrêt propre |

**Concepts Rust rencontrés :**

- **Les signaux et le « processus n° 1 ».** Pour arrêter un conteneur, Docker envoie le signal
  `SIGTERM`, puis tue le programme au bout de quelques secondes s'il tourne encore. Dans un
  conteneur, notre programme est le processus n° 1, que le noyau ne tue jamais d'office : sans
  code pour écouter `SIGTERM`, il l'ignorait, et Docker finissait par le tuer brutalement, sans
  enregistrer les derniers états. `tokio::signal::unix::signal(SignalKind::terminate())` écoute
  maintenant ce signal, et l'arrêt prend 0,3 seconde au lieu de 10.
- **`#[cfg(unix)]`, la compilation conditionnelle.** Windows n'a pas de `SIGTERM`. Le bloc
  marqué `#[cfg(unix)]` n'est compilé que sous Linux et macOS, et le bloc `#[cfg(not(unix))]`
  ailleurs. Un seul code source, et la CI vérifie les deux (Linux et Windows).
- **`take` pour borner une lecture.** Le contrôle de santé ne lit que les 64 premiers octets de
  la réponse (`stream.take(64)`) : même un faux serveur bavard ne peut pas lui faire remplir la
  mémoire.
- **`IsTerminal`.** Les couleurs des logs sont des codes spéciaux. Dans un terminal, ils
  deviennent des couleurs ; dans les logs de Docker, des caractères illisibles.
  `std::io::stdout().is_terminal()` permet de n'en mettre que quand quelqu'un regarde un vrai
  terminal.

## Testé

- L'image se construit, et le conteneur passe « healthy » en quelques secondes.
- Le tableau de bord répond avec tous ses en-têtes de sécurité.
- Création d'un compte par `docker compose run`, connexion, lecture des états.
- `docker compose stop` : arrêt propre en 0,3 s (code de sortie 0), derniers états enregistrés,
  et **restaurés** au redémarrage.
- Les automatisations de `config/automations.yaml` sont chargées.
- Sauvegarde, puis restauration : le serveur redémarre « healthy », avec ses états et ses
  comptes. (Le premier essai, avec `docker cp`, cassait la base : c'est ce qui a mené à l'outil
  `sauvegarde`.)
- Dans l'image : ni `/bin/sh` ni `curl` ; fuseau horaire Europe/Paris et certificats racines
  présents (pour la création du jeton Somfy).
- La CI refait ces vérifications à chaque modification (job « image Docker »).

Tailscale n'a pas pu être testé ici : il demande ton compte. Si `tailscale serve` affiche une
erreur, copie-la-moi.
