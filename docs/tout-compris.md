# Étape 83 : toute la maison en une commande, expliquée

## Le but

Avec Home Assistant OS, on installe le système, et tout est là : le facteur MQTT, la voix, les
modules (« add-ons »). Ici aussi, désormais. Sur le [mini-PC](mini-pc.md) :

```bash
sudo ./installer.sh --sonos --ring
```

installe et règle **toute la maison**, sans rien à copier à la main :

| | Toujours | Avec `--sonos` | Avec `--ring` | Avec `--voix` |
| --- | :---: | :---: | :---: | :---: |
| Le serveur, son service, sa mise à jour de chaque nuit | ✓ | | | |
| Les mises à jour de sécurité de Debian, automatiques | ✓ | | | |
| Mosquitto (le facteur MQTT), réglé, avec ses comptes | ✓ | | | |
| Les enceintes Sonos | | ✓ | | |
| Piper et la voix française | | ✓ | | ✓ |
| ring-mqtt, pour la [sonnette Ring](sonnette.md) | | | ✓ | |
| Parakeet, pour [parler à la maison](voix.md) depuis la tablette | | | | ✓ |

Il ne reste à faire que ce qui demande **toi** : créer ton compte (`sudo ha-maison user add david`),
connecter le compte Ring (le code arrive par SMS), relier les appareils qui ont un bouton à presser
(Hue, Somfy, Iglou…).

## Ce qui est fait, et comment

**Les mises à jour de Debian** : le paquet `unattended-upgrades`, réglé pour installer chaque jour
les correctifs de sécurité.

**Mosquitto** (le facteur [MQTT](etape-4-mqtt.md)) :
- il n'écoute que sur **le mini-PC lui-même** (`127.0.0.1`) : rien d'autre sur le réseau ne peut s'y
  connecter ;
- **aucun accès anonyme**, un compte par programme (`ha-server`, et `ring` avec `--ring`), chacun avec
  un mot de passe de **32 caractères tirés au hasard**, rangé dans `/etc/ha-maison/secrets` (le
  service le lit, personne d'autre) ;
- le serveur est réglé pour s'en servir. Sans MQTT, il montrait des appareils simulés : avec, il
  montre les vrais.

**La voix** (`--sonos`) : Piper et la voix `fr_FR-siwis-medium`, téléchargés dans
`/opt/ha-maison/piper` et **vérifiés par leur empreinte** (SHA-256 écrite dans le script) : un
fichier modifié en route est refusé, rien n'est installé. Les enceintes sont cherchées sur le réseau.

**ring-mqtt** (`--ring`) : Docker, puis le service `ha-maison-ring`, qui lance ring-mqtt dans un
conteneur à une version **épinglée par son empreinte** (`tsightler/ring-mqtt:5.9.3@sha256:…`). Le
conteneur ne peut pas changer de version tout seul ; la nouvelle arrive avec une mise à jour du
serveur, téléchargée, puis l'ancienne est effacée.

**Ce que l'installation règle pour le serveur** va dans `/etc/ha-maison/ha-maison.auto.env`, réécrit
à chaque fois. **Tes réglages passent devant** : une ligne dans `ha-maison.env` l'emporte toujours.
Si tu as ton propre broker (`HA_MQTT_HOST` dans `ha-maison.env`), Mosquitto n'est pas installé.

## Les options sont retenues

Elles sont écrites dans `/etc/ha-maison/options`. La mise à jour de chaque nuit relance
l'installation sans option : elle les relit, et garde Sonos et la sonnette. Pour en retirer une
(par exemple la sonnette), enlève sa ligne de ce fichier, puis `sudo ./installer.sh` : le service
`ha-maison-ring` est arrêté (son jeton Ring reste, au cas où).

Chaque étape est sautée si elle est déjà faite : la relancer ne télécharge rien, ne change aucun mot
de passe et ne redémarre que ce qui a changé.

## Sécurité

- **Mosquitto fermé au réseau** : seulement `127.0.0.1`, pas d'anonyme, un compte par programme.
- **Les mots de passe** ne sont jamais écrits dans un réglage ni affichés : tirés au hasard
  (`/dev/urandom`), rangés dans des fichiers lisibles seulement par le service qui s'en sert (et
  l'administrateur). Celui de ring-mqtt est dans son `config.json`, lisible par l'administrateur seul.
- **Rien n'est installé sans être vérifié** : les paquets par Debian (leurs signatures), la voix par
  son empreinte, ring-mqtt par l'empreinte de son image.
- **Le conteneur de ring-mqtt** a accès au réseau du mini-PC (il doit joindre les serveurs de Ring et
  Mosquitto), pas aux fichiers du serveur : seulement à son dossier `/var/lib/ring-mqtt`.
- **Docker n'est installé qu'avec `--ring`.** Le serveur lui-même ne tourne jamais dans Docker.

## Comment c'est construit

| Fichier | Rôle |
| ------- | ---- |
| [packaging/linux/tout-compris.sh](../packaging/linux/tout-compris.sh) | Tout ce qu'il faut autour du serveur, étape par étape, chacune sautée si elle est déjà faite |
| [packaging/linux/installer.sh](../packaging/linux/installer.sh) | Les options (`--sonos`, `--ring`, `--voix`, retenues), puis appelle `tout-compris.sh` |
| [packaging/linux/ha-maison-ecoute.service](../packaging/linux/ha-maison-ecoute.service) | Le service d'écoute (Parakeet), avec `--voix` ([étape 84](voix.md)) |
| [packaging/linux/ha-maison-ring.service](../packaging/linux/ha-maison-ring.service) | Le service de ring-mqtt, avec l'image épinglée |
| [packaging/linux/ha-maison.service](../packaging/linux/ha-maison.service) | Lit `ha-maison.auto.env`, puis `ha-maison.env` (qui passe devant) |
| [packaging/linux/ha-maison](../packaging/linux/ha-maison) | La commande d'administration lit les deux aussi |

Essayé dans un Debian 13 avec systemd, depuis le dossier de l'archive :
- l'installation sans option : mises à jour de Debian, Mosquitto sur `127.0.0.1` seulement, le
  serveur connecté avec son mot de passe ; une connexion anonyme refusée ;
- `--sonos --ring` : la voix (téléchargée, vérifiée, « On sonne à la porte. » prononcée en 0,8 s),
  Docker, ring-mqtt démarré, sa page de connexion à Ring ouverte, les mots de passe et les dossiers
  avec les bons droits ;
- relancée sans option : Sonos et la sonnette gardés, rien retéléchargé ;
- une nouvelle version de ring-mqtt : le service redémarré ;
- la machine redémarrée : Mosquitto, Docker, le serveur (reconnecté à MQTT), ring-mqtt, la mise à
  jour de chaque nuit et celles de Debian repartent seuls.

### Concepts utilisés

**Une image épinglée par son empreinte** : un nom comme `ring-mqtt:5.9.3` peut être déplacé vers
une autre image par son auteur (ou par quelqu'un qui aurait volé son compte). L'empreinte
(`@sha256:…`) désigne une image **exacte** : si le moindre octet change, Docker la refuse.

**L'ordre des fichiers de réglages** : systemd lit `ha-maison.auto.env`, puis `ha-maison.env`. Pour
un même réglage, le dernier lu gagne : le tien.
