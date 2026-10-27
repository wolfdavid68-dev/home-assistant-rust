# Installer le serveur chez soi

Trois façons, selon la machine :

| Machine | Ce qu'on télécharge | Mise à jour |
| --- | --- | --- |
| **Windows** | `ha-maison-vX.Y.Z-windows.zip` : le programme et ses fichiers | remplacer `ha-server.exe` |
| **Linux** (Ubuntu, Debian…) | `compose.yaml` : Docker télécharge l'image tout seul | `docker compose pull && docker compose up -d` |
| **Mini-PC dédié** sous Debian | `ha-maison-linux-x86_64.tar.gz` : le programme et son installateur | automatique, chaque nuit |

Tous sont sur la page des versions :
<https://github.com/wolfdavid68-dev/home-assistant-rust/releases>.

Le **mini-PC dédié** (une machine qui ne fait que la maison et démarre toute seule) a son propre
guide, de l'installation de Debian à la première connexion : [mini-pc.md](mini-pc.md).

Avec Windows ou Docker, le tableau de bord s'ouvre à l'adresse <http://127.0.0.1:8123/>, sur le PC
lui-même. Pour l'ouvrir depuis un téléphone de la maison, voir « Depuis le téléphone » plus bas.

## Windows

### 1. Ranger le dossier

Télécharge le zip, puis décompresse-le dans **ton profil**, par exemple :

```text
C:\Users\<toi>\AppData\Local\ha-maison
```

Ton profil est protégé : seul ton compte Windows peut y lire. C'est là que vivront la base
(`ha.db`) et la clé qui chiffre les jetons Hue et Somfy (`ha.key`). Évite `C:\` ou le Bureau
partagé.

### 2. Créer ton compte

Dans l'Explorateur, ouvre le dossier. Tape `cmd` dans la barre d'adresse, puis Entrée. Dans la
fenêtre noire :

```bat
ha-server.exe user add david
```

Le mot de passe est demandé deux fois. Il faut 12 caractères au moins : une phrase de plusieurs
mots convient très bien.

### 3. Démarrer

Double-clique sur **`demarrer.bat`**. Une fenêtre noire affiche le journal du serveur : laisse-la
ouverte. Ouvre ensuite <http://127.0.0.1:8123/> dans le navigateur et connecte-toi.

Pour arrêter le serveur, ferme la fenêtre, ou tape Ctrl+C dedans.

### 4. Démarrer avec Windows

Double-clique sur **`demarrage-automatique.bat`** : le serveur se lancera, fenêtre réduite, à
chaque ouverture de ta session. Pour l'enlever :

```bat
schtasks /Delete /TN "Maison ha-server" /F
```

Le serveur ne tourne donc que lorsque ta session est ouverte. Pour qu'il tourne même sans
session, le PC allumé au démarrage, il faut un service Windows. Dis-le-moi le jour où tu en as
besoin.

### 5. Les réglages

Ils sont dans `demarrer.bat`, à ouvrir avec le Bloc-notes (clic droit, puis Modifier). Chaque
ligne `set NOM=valeur` est un réglage, et `rem` met une ligne en commentaire. Par exemple, pour le
soleil, la météo et la protection solaire :

```bat
set HA_LATITUDE=48.85
set HA_LONGITUDE=2.35
```

Les fichiers `*.example.yaml` sont des modèles. Pour en utiliser un, copie-le sans `.example`
(par exemple `areas.yaml`), adapte-le, puis vérifie-le :

```bat
ha-server.exe areas check
ha-server.exe protection-solaire check
```

Redémarre ensuite le serveur pour qu'il le lise.

### Mettre à jour

Arrête le serveur, remplace `ha-server.exe` par celui du nouveau zip, puis redémarre. La base,
la clé, tes réglages et tes `.yaml` ne bougent pas.

## Linux, avec Docker

Il faut Docker et son extension Compose (`docker compose version` doit répondre). Sur Ubuntu ou
Debian, le plus simple est le script officiel : <https://docs.docker.com/engine/install/>.

### 1. Installer

```bash
mkdir ~/ha-maison && cd ~/ha-maison
curl -LO https://github.com/wolfdavid68-dev/home-assistant-rust/releases/latest/download/compose.yaml
mkdir config sauvegardes
docker compose up -d
```

### 2. Créer ton compte

```bash
docker compose run --rm ha-server user add david
```

Ouvre ensuite <http://127.0.0.1:8123/>.

### 3. Les réglages

- **Les variables :** elles sont dans `compose.yaml`, sous `environment:`. Enlève le `#` devant
  une ligne pour l'activer.
- **Tes fichiers `.yaml` :** ils vont dans le dossier `config/`, avec les mêmes modèles que dans
  le dépôt. Pour en vérifier un :
  ```bash
  docker compose run --rm ha-server areas check
  ```
- **La prise en compte :** après un changement, lance `docker compose up -d`.

### Mettre à jour

```bash
docker compose pull && docker compose up -d
```

Pour rester sur une version précise, remplace `latest` par son numéro dans `compose.yaml` (par
exemple `0.1.0`).

### Essayer les nouveautés avant une version

À chaque fusion sur `main`, GitHub fabrique et publie une image de test : `main` (et
`main-<commit>`, pour revenir à une image précise), prête 2 à 4 minutes après la fusion (les
dépendances compilées sont gardées d'une fois sur l'autre). Rien à compiler chez soi :

1. dans `compose.yaml`, remplace `:latest` par `:main` ;
2. à chaque nouveauté :
   ```bash
   docker compose pull && docker compose up -d
   ```

Elle est démarrée et vérifiée avant d'être publiée, comme celle des versions, mais ce n'est
pas tout à fait la même : pour être prête vite, elle est compilée avec le profil `essai`
(`Cargo.toml`), sans l'optimisation « tout le programme d'un coup » (LTO) des versions. Le
programme fait la même chose, à peine moins vite, et l'image peut contenir une nouveauté pas
encore essayée chez toi. Pour revenir aux versions, remets `:latest`. La base, la clé et tes
réglages ne bougent pas.

La première fois que l'image `main` est publiée, vérifie sur GitHub (ton profil, onglet
**Packages**, `home-assistant-rust`, **Package settings**) qu'elle est **publique** : sinon
`docker compose pull` demanderait un compte.

### Garder une image `main` précise

`main` change à chaque fusion. Pour rester sur une image que tu as essayée, et savoir
exactement à laquelle revenir, mets son commit dans `compose.yaml` : chaque image `main` est
aussi publiée sous `main-<commit>` (les 7 premiers caractères du commit, visibles sur la page
du dépôt ou dans `git log --oneline`).

1. dans `compose.yaml` :
   ```yaml
   image: ghcr.io/wolfdavid68-dev/home-assistant-rust:main-1a2b3c4
   ```
2. puis :
   ```bash
   docker compose pull && docker compose up -d
   ```

Pour revenir en arrière, remets dans `compose.yaml` l'étiquette d'avant (note-la avant de la
changer) et relance les deux mêmes commandes. `docker compose ps` et `docker compose logs -f`
montrent si le service est reparti (`healthy`).

### Construire l'image sur sa machine

Pour essayer une branche qui n'est pas encore sur `main`, construis l'image toi-même depuis le
dépôt. Le `compose.yaml` du dépôt utilise déjà l'image `ha-server:local`. Sous Windows (Docker
Desktop), `scripts/deployer.ps1` fait tout dans l'ordre :

```powershell
.\scripts\deployer.ps1 -Dossier <dossier du compose.yaml> -Image ha-server:local
```

(`-Dossier .` si tu utilises le `compose.yaml` du dépôt, depuis le dossier du dépôt.)

1. l'image en service est gardée sous une étiquette de secours, `ha-server:local-avant-<date>` ;
2. la nouvelle image est construite depuis le dépôt (`docker build`) ;
3. le service `ha-server` redémarre avec elle (`docker compose up -d --no-build ha-server`) ;
4. le script attend qu'il soit `healthy` (3 minutes au plus, `-Delai` pour changer) ;
5. s'il ne l'est pas, l'image de secours reprend sa place et le service redémarre avec.

À la fin, il affiche la commande pour revenir en arrière à la main et celle pour supprimer
l'image de secours quand tu n'en as plus besoin (`docker image rm …-avant-<date>`). La base, la
clé et tes réglages ne sont pas touchés. Si Windows refuse de lancer le script, autorise-le
pour cette fenêtre seulement : `Set-ExecutionPolicy -Scope Process Bypass`.

Sous Linux, la même chose à la main, dans le dossier du dépôt :

```bash
docker tag ha-server:local ha-server:local-avant
docker compose up -d --build ha-server
# si ça ne repart pas :
docker tag ha-server:local-avant ha-server:local && docker compose up -d --no-build ha-server
```

## Depuis le téléphone

Par défaut, le serveur n'écoute que sur le PC lui-même (`127.0.0.1`). Pour l'ouvrir à ton
réseau local :

- **Windows :**
  - dans `demarrer.bat`, mets `set HA_BIND=0.0.0.0:8123` ;
  - au premier démarrage, Windows demande l'accès au réseau : coche **réseaux privés**
    seulement.
- **Linux :** crée un fichier `.env` à côté de `compose.yaml`, avec la ligne
  `HA_PUBLISH=192.168.1.20` (l'adresse du PC).

Le tableau de bord est alors à `http://<adresse du PC>:8123/` depuis le téléphone, en Wi-Fi à
la maison.

**Ne redirige jamais ce port depuis ta box Internet.** Pour y accéder hors de la maison, passe
par un VPN (Tailscale ou WireGuard) : voir le [déploiement](etape-7-deploiement.md) et
[SECURITY.md](../SECURITY.md).

## Les sauvegardes

- **Windows :** `demarrer.bat` les active d'office. Chaque nuit, une copie vérifiée de la base
  va dans le dossier `sauvegardes`, et les 7 dernières sont gardées.
- **Linux :** même chose, dans le dossier `sauvegardes/` à côté de `compose.yaml`.

**Mets `ha.key` à l'abri ailleurs** (une clé USB, un gestionnaire de mots de passe). Sans elle,
une sauvegarde ne rend pas les jetons Hue et Somfy, et il faudrait réassocier les box. Voir les
[sauvegardes](sauvegardes.md).

## Publier une version (pour le dépôt)

1. Mettre `version` à jour dans `Cargo.toml`, puis fusionner sur `main`.
2. Avec l'accord de David, poser l'étiquette :
   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```
   Ou, sans ligne de commande, depuis la page des versions de GitHub : **Draft a new release** →
   **Choose a tag** : taper `v0.1.0`, puis **Create new tag: v0.1.0 on publish** → **Target** :
   `main` → **Publish release**. Le workflow complète alors cette version (les fichiers arrivent
   en une dizaine de minutes).
3. Le workflow `release.yml` fait le reste :
   - la version GitHub, avec ses notes ;
   - le zip Windows, compilé et vérifié (`ha-server.exe --version`) ;
   - l'image Docker, testée puis publiée sur `ghcr.io` (`0.1.0` et `latest`) ;
   - le `compose.yaml` joint à la version.

La première fois, vérifie sur GitHub (ton profil, onglet **Packages**) que l'image
`home-assistant-rust` est bien **publique**. Sinon, `docker compose pull` demanderait un compte.
