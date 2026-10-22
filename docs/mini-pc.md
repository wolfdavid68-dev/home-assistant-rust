# Étape 77 : le mini-PC dédié à la maison, expliqué

## Le but

Une machine qui ne fait **que** la maison, comme une box : on la branche, elle démarre toute seule
et relance le serveur, même après une coupure de courant. Pas d'écran, pas de Windows à mettre à
jour, pas de session à ouvrir. On s'en sert depuis le téléphone ou l'ordinateur.

C'est l'idée de Home Assistant OS. Ici, la machine tourne sous **Debian**, un Linux stable et léger,
et le serveur est un **service** : le système le démarre, le surveille et le relance.

## Ce qu'il faut

| | Conseillé | Pourquoi |
| --- | --- | --- |
| Mini-PC | 64 bits (Intel ou AMD), type Intel N100 | le programme est fait pour ces processeurs |
| Mémoire | 8 Go (1 Go suffirait) | le serveur prend ~20 Mo, Debian ~250 Mo, la voix ~200 Mo quand elle parle |
| Disque | SSD, 256 Go | pas de carte SD : elle s'use avec la base, et lâche sans prévenir |
| Réseau | câble Ethernet vers la box | plus fiable que le Wi-Fi pour une machine qui tourne jour et nuit |
| Pour l'installation | une clé USB (2 Go ou plus), un écran et un clavier | seulement le temps d'installer Debian |

> **Attention :** installer Debian **efface tout le SSD**, Windows compris. Copie avant ce que tu
> veux garder.

## 1. Préparer la clé d'installation

Sur ton PC habituel :

1. Télécharge Debian sur <https://www.debian.org/download> : le fichier
   `debian-13.…-amd64-netinst.iso` (environ 700 Mo).
2. Télécharge **balenaEtcher** (<https://etcher.balena.io>) ou **Rufus** (<https://rufus.ie>).
3. Copie le fichier `.iso` sur la clé avec l'un des deux (la clé est effacée). Avec Rufus, si on
   te le demande, choisis le mode **DD**.

## 2. Régler le mini-PC (BIOS)

Branche l'écran, le clavier, le câble réseau et la clé, puis allume. Appuie tout de suite sur la
touche du BIOS (souvent **Suppr**, **F2** ou **Échap** : elle s'affiche au démarrage).

- **Redémarrer après une coupure de courant** : cherche « Restore on AC Power Loss », « AC Power
  Recovery » ou « State after G3 », et mets **Power On**. C'est ce qui rend la machine autonome.
- **Démarrer sur la clé** : dans « Boot », mets la clé USB en premier. (Ou, au démarrage, la touche
  du menu de démarrage : souvent **F7**, **F11** ou **F12**.)

Enregistre et quitte (souvent **F10**).

## 3. Installer Debian

Choisis **Graphical install**, puis :

| Écran | Réponse |
| --- | --- |
| Langue, pays, clavier | Français, France, Français |
| Nom de la machine | `maison` |
| Domaine | laisse vide |
| Mot de passe du superutilisateur (root) | **laisse vide** (deux fois) : ton compte pourra alors tout faire avec `sudo` |
| Ton nom, ton identifiant, ton mot de passe | par exemple `david`, avec une phrase de passe solide |
| Partitionnement | « Assisté – utiliser un disque entier », le SSD, « Tout dans une seule partition », puis « Oui » pour écrire |
| Sélection des logiciels | **décoche** « environnement de bureau Debian » et « GNOME » ; **coche** « serveur SSH » et « utilitaires usuels du système » |
| Programme de démarrage GRUB | Oui, sur le SSD |

À la fin, retire la clé : la machine redémarre sur Debian. Tu peux débrancher l'écran et le clavier.

## 4. Lui donner une adresse fixe

Le tableau de bord sera à l'adresse du mini-PC : elle ne doit pas changer.

1. Sur la page de ta box, trouve l'appareil `maison` et son adresse (par exemple `192.168.1.20`).
2. Réserve-lui cette adresse. Sur une Freebox : **Freebox OS → Paramètres de la Freebox → DHCP →
   Baux statiques**, ajoute l'appareil `maison`.

Ensuite, tout se fait depuis ton PC, dans le Terminal Windows (ou PowerShell) :

```bash
ssh david@192.168.1.20
```

La première fois, réponds `yes`, puis ton mot de passe Debian.

## 5. Les mises à jour de sécurité de Debian

Rien à faire : l'installation du serveur (étape suivante) les rend automatiques. Debian installe
alors chaque jour, tout seul, les correctifs de sécurité.

## 6. Installer le serveur, et tout ce qu'il lui faut

```bash
wget https://github.com/wolfdavid68-dev/home-assistant-rust/releases/latest/download/ha-maison-linux-x86_64.tar.gz
tar -xzf ha-maison-linux-x86_64.tar.gz
cd ha-maison-v*-linux-x86_64
sudo ./installer.sh --sonos --ring
```

Comme Home Assistant OS, une seule commande installe **toute la maison** ([étape 83](tout-compris.md)) :
le serveur, les mises à jour de Debian, le facteur MQTT (Mosquitto, réglé avec des mots de passe
tirés au hasard), et avec les options :

- **`--sonos`** : les enceintes Sonos et la voix des annonces (Piper, en français) ;
- **`--ring`** : la sonnette Ring (ring-mqtt, voir [la sonnette](sonnette.md)).

Sans enceintes ni sonnette, `sudo ./installer.sh` tout court. Les options sont retenues : la mise à
jour de chaque nuit les garde.

Le script dit ce qu'il fait, démarre le serveur et donne l'adresse du tableau de bord. Puis crée
ton compte (le mot de passe est demandé deux fois, 12 caractères au moins) :

```bash
sudo ha-maison user add david
```

Le tableau de bord est en **HTTPS** (étape 79) : les mots de passe ne circulent jamais en clair sur
le Wi-Fi. Sur chaque téléphone, tablette ou PC, installe une fois le **certificat de la maison** :
ouvre <http://192.168.1.20:8123/maison-ca.crt>, puis suis [le guide du HTTPS](https-maison.md)
(deux minutes par appareil).

Ouvre ensuite <https://192.168.1.20:8123/> : c'est prêt. Le serveur redémarrera tout seul à chaque
allumage, et **se mettra à jour tout seul** chaque nuit (voir plus bas).

## Au quotidien

**La commande `ha-maison`** remplace `ha-server` de Windows : mêmes commandes, avec `sudo` devant.

```bash
sudo ha-maison user list            # les comptes
sudo ha-maison hue pair 192.168.1.10
sudo ha-maison somfy pair 192.168.1.11
sudo ha-maison timetree pair
sudo ha-maison sonos                # les enceintes trouvées
sudo ha-maison backup list          # les sauvegardes
```

**Le service** :

```bash
sudo systemctl status ha-maison     # est-il en marche ?
sudo systemctl restart ha-maison    # relire les réglages
sudo journalctl -u ha-maison -f     # le journal, en direct (Ctrl+C pour sortir)
```

**Les réglages** sont dans `/etc/ha-maison/ha-maison.env` : une ligne par réglage, enlève le `#`
pour l'activer.

```bash
sudo nano /etc/ha-maison/ha-maison.env    # Ctrl+O pour enregistrer, Ctrl+X pour quitter
sudo systemctl restart ha-maison
```

**Tes fichiers** (pièces, automatisations, scènes…) vont à côté, dans `/etc/ha-maison`. Des
exemples y sont déjà :

```bash
sudo cp /etc/ha-maison/areas.example.yaml /etc/ha-maison/areas.yaml
sudo nano /etc/ha-maison/areas.yaml
sudo ha-maison areas check
sudo systemctl restart ha-maison
```

Les fichiers secrets (clé Météo-France, mot de passe MQTT) vont dans `/etc/ha-maison/secrets` :
seuls le service et `sudo` peuvent les lire.

**Si le tableau de bord ne répond plus** après un changement de réglage, le journal dit pourquoi :

```bash
sudo journalctl -u ha-maison -n 20
```

Par exemple « il faut HA_LATITUDE et HA_LONGITUDE ensemble ». Corrige, puis `restart` : le service
réessaie de toute façon toutes les 5 secondes.

## Les mises à jour du serveur, automatiques

Chaque nuit vers 4 h (après la sauvegarde de 3 h 30), le mini-PC regarde s'il existe une version
plus récente du serveur, et l'installe tout seul. Si la machine était éteinte à 4 h, il le fait au
prochain allumage.

**Prudemment :**

1. Il lit d'abord l'**empreinte** (SHA-256) de la dernière archive publiée, un tout petit fichier :
   si c'est celle déjà vue, rien n'est téléchargé.
2. Il télécharge l'archive depuis GitHub (HTTPS) et **vérifie son empreinte** : une archive
   abîmée ou modifiée en route est refusée, rien n'est installé.
3. Il n'installe qu'une version **plus récente** : jamais de retour en arrière.
4. Il **sauvegarde la base**, garde l'ancien programme, puis installe (comme `installer.sh`).
5. Si le nouveau serveur **ne répond pas** dans les 30 secondes, il remet **tout seul** l'ancien
   programme et la base d'avant la mise à jour, et ne retente plus cette version : il attend la
   suivante.

Tes réglages, tes comptes, l'historique et les appareils associés restent. La version installée
s'affiche dans **Mon compte → Système**.

**Ce qu'elle a fait** :

```bash
sudo journalctl -u ha-maison-maj          # par exemple « mise à jour : 0.1.0 -> 0.1.1 »
systemctl list-timers ha-maison-maj.timer  # la prochaine fois
```

**La lancer tout de suite** : `sudo systemctl start ha-maison-maj`.

**L'arrêter** (pour mettre à jour toi-même) :

```bash
sudo systemctl disable --now ha-maison-maj.timer
```

Elle reste arrêtée, même après une mise à jour à la main. Pour la remettre :
`sudo systemctl enable --now ha-maison-maj.timer`.

### À la main

Les mêmes commandes que pour l'installation : la nouvelle archive, puis `sudo ./installer.sh`.

```bash
cd ~
rm -rf ha-maison-v*-linux-x86_64 ha-maison-linux-x86_64.tar.gz
wget https://github.com/wolfdavid68-dev/home-assistant-rust/releases/latest/download/ha-maison-linux-x86_64.tar.gz
tar -xzf ha-maison-linux-x86_64.tar.gz
cd ha-maison-v*-linux-x86_64
sudo ./installer.sh
```

Le programme est remplacé et le service redémarre. Tes réglages, tes comptes, l'historique et les
appareils associés restent.

## Les sauvegardes

Le serveur sauvegarde la base chaque nuit dans `/var/lib/ha-maison-sauvegardes` (les 7 dernières).
Elles sont sur le même SSD : copie-les ailleurs de temps en temps. Sur le mini-PC :

```bash
sudo sh -c "cp /var/lib/ha-maison-sauvegardes/ha-*.db $HOME/ && chown $(id -un) $HOME/ha-*.db"
```

(`sudo sh -c` : seul l'administrateur peut lire le dossier des sauvegardes, même pour trouver
les noms des fichiers.)

Puis, depuis le Terminal de ton PC : `scp "david@192.168.1.20:ha-*.db" .`

**La clé** qui chiffre les jetons des appareils (Hue, Somfy, TimeTree) n'est volontairement pas
dans les sauvegardes. Garde-la une fois pour toutes dans ton gestionnaire de mots de passe :

```bash
sudo cat /var/lib/ha-maison-cle/ha.key
```

Restaurer une sauvegarde (voir [les sauvegardes](sauvegardes.md)) :

```bash
sudo systemctl stop ha-maison
sudo ha-maison backup restore /var/lib/ha-maison-sauvegardes/ha-AAAA-MM-JJ_HHMMSS.db
sudo systemctl start ha-maison
```

## La voix des annonces (Sonos)

Avec `sudo ./installer.sh --sonos`, Piper et la voix française (`fr_FR-siwis-medium`) sont
téléchargés dans `/opt/ha-maison/piper` et **vérifiés par leur empreinte** (une version modifiée en
route est refusée), et les enceintes sont cherchées sur le réseau (`HA_SONOS=auto`). Pour essayer la
voix : `sudo ha-maison voix "Bonjour"` (écrit `/var/lib/ha-maison/essai-voix.wav`).

Le serveur lui-même ne tourne pas dans Docker : la recherche des enceintes (`HA_SONOS=auto`) et Alexa
en local marchent directement. (Seul ring-mqtt, avec `--ring`, a son conteneur.)

## Sécurité

**Ce que le service peut faire** (le fichier `ha-maison.service`) : le minimum.

| Protection | Effet |
| --- | --- |
| Compte `ha-maison`, sans mot de passe ni shell | le serveur ne tourne jamais en administrateur |
| `ProtectSystem=strict`, `ProtectHome=yes` | il ne peut écrire que dans ses trois dossiers de `/var/lib`, et ne voit pas `/home` |
| `StateDirectoryMode=0700`, `UMask=0077` | la base, la clé et les sauvegardes ne sont lisibles que par lui |
| `NoNewPrivileges`, `CapabilityBoundingSet` | un seul droit spécial : écouter sur le port 80 (Alexa) |
| `DevicePolicy=closed` | aucun périphérique, sauf les ports série USB (module Linky) |
| `SystemCallFilter=@system-service` | les appels système d'un service ordinaire, rien de plus |
| `MemoryMax=1G`, `TasksMax=256` | un emballement ne peut pas bloquer la machine |

`systemd-analyze security ha-maison` lui donne **1,8** (« OK », sur une échelle où 10 est un
programme sans aucune protection).

**Les réglages** (`/etc/ha-maison`) appartiennent à `root` : le service les lit, mais ne peut pas
les modifier. Même un serveur piraté ne pourrait pas changer ses propres réglages.

**Le réseau** : le tableau de bord écoute sur le réseau de la maison (`HA_BIND=0.0.0.0:8123`), en
HTTPS ([étape 79](https-maison.md)), puisqu'il n'y a pas d'écran sur la machine. La box le protège d'Internet : **ne redirige jamais**
le port 8123 depuis la box. Pour l'accès à distance, un VPN (voir l'[étape 7](etape-7-deploiement.md)).
SSH (port 22) ne sert qu'à l'administration ; pour aller plus loin, une clé SSH à la place du mot de
passe.

**Ce que l'installation ajoute** ([étape 83](tout-compris.md)) : Mosquitto n'écoute que sur
`127.0.0.1` (le mini-PC lui-même), sans compte anonyme, un compte par programme avec un mot de passe
de 32 caractères tirés au hasard, rangé dans `/etc/ha-maison/secrets` ; la voix est vérifiée par son
empreinte ; ring-mqtt tourne dans un conteneur à la version épinglée par son empreinte.

**Les mises à jour** : Debian tout seul pour le système (étape 5), et la mise à jour de chaque nuit
pour le serveur. Elle tourne en administrateur, puisqu'elle remplace le programme : elle fait
donc confiance à GitHub et à ce dépôt. L'empreinte vient du même endroit que l'archive : elle
protège d'un téléchargement abîmé ou modifié en route, pas d'un compte GitHub piraté. C'est pour
cela qu'une version n'est publiée qu'avec l'accord de David (voir `CLAUDE.md`). Pour ne rien
installer sans l'avoir vu, arrête-la (plus haut) et mets à jour à la main.

## Comment c'est construit

- **`packaging/linux/installer.sh`** : vérifie la machine (root, systemd, 64 bits, le programme
  démarre), crée le compte, copie le programme, les réglages (seulement s'ils n'existent pas) et le
  service, démarre, puis attend que le serveur réponde (`ha-server healthcheck`). Relancé, il met à
  jour sans rien perdre.
- **`packaging/linux/tout-compris.sh`** : tout ce qu'il faut autour du serveur (mises à jour de
  Debian, Mosquitto, la voix, ring-mqtt), lu par `installer.sh` ([étape 83](tout-compris.md)).
- **`packaging/linux/ha-maison.service`** : le service systemd et ses protections.
- **`packaging/linux/ha-maison.env`** : les réglages, avec les chemins de Linux.
- **`packaging/linux/ha-maison`** : la commande d'administration. Elle lance `ha-server` sous le
  compte du service, avec ses réglages.
- **`packaging/linux/mise-a-jour.sh`**, **`ha-maison-maj.service`** et **`ha-maison-maj.timer`** :
  la mise à jour de chaque nuit. Ce qu'elle retient (la dernière empreinte vue, une version
  refusée, l'ancien programme) est dans `/var/lib/ha-maison-maj`, lisible par root seul.
- **`.github/workflows/release.yml`** : à chaque version, l'archive `ha-maison-linux-x86_64.tar.gz`
  est compilée dans Debian 12 (le programme tourne sur Debian 12 et 13), puis jointe à la version
  avec son empreinte (`.sha256`, envoyée après l'archive).
- **CI** : `shellcheck` relit les quatre scripts à chaque PR.

Essayé dans un Debian 13 avec systemd, depuis l'archive :
- l'installation, le compte et la connexion ;
- le mot de passe tapé dans un vrai terminal ;
- un redémarrage de la machine (le service repart seul) ;
- une mise à jour à la main (réglages, compte et données gardés) ;
- la mise à jour automatique, avec une fausse page des versions :
  - même version publiée : rien à faire, puis plus aucun téléchargement ;
  - une version 0.1.1 : installée, compte et données gardés ;
  - une version 0.1.2 qui ne démarre pas : retour tout seul à 0.1.1 avec la base d'avant, et
    pas de nouvel essai la nuit suivante ;
  - une archive modifiée : refusée (empreinte fausse) ;
  - une version plus ancienne : ignorée ;
  - arrêtée, puis l'installateur relancé : elle reste arrêtée ;
- une sauvegarde et un fichier de pièces ;
- la voix Piper et l'écoute sur le port 80, avec toutes les protections du service.

Le serveur y prenait **19 Mo** de mémoire.

### Concepts utilisés

**systemd, le chef d'orchestre de Linux** : il démarre les services dans l'ordre, les relance
s'ils tombent (`Restart=on-failure`) et range leur journal (`journalctl`). Un service est un petit
fichier texte (`.service`) ; `systemctl enable` le fait démarrer à chaque allumage.

**`StateDirectory=`** : systemd crée lui-même les dossiers de données, avec le bon propriétaire et
les bons droits. Avec `ProtectSystem=strict`, ce sont les seuls endroits où le service peut écrire.

**Un minuteur systemd (`.timer`)** : l'équivalent de « tous les jours à 4 h ». `Persistent=true`
rattrape une nuit où la machine était éteinte, et `RandomizedDelaySec` étale l'heure, pour que
tous les mini-PC ne téléchargent pas à la même seconde.

**Un script lu en entier avant de commencer** : `sh` lit un script ligne à ligne pendant qu'il
tourne. La mise à jour remplace son propre fichier : tout son travail est donc dans une fonction
`main`, que `sh` lit en entier avant de l'exécuter.

**`runuser`** : lance une commande sous un autre compte, même un compte sans shell ni mot de passe
comme `ha-maison`. La commande `ha-maison` s'en sert pour que `user add` écrive dans la base avec
les mêmes droits que le service.
