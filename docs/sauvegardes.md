# Étape 18 : les sauvegardes automatiques, expliqué

## Le but

Une carte SD qui lâche, un disque qui meurt, une fausse manipulation… et ce sont tes comptes,
tes automatisations de l'éditeur, l'historique et les associations d'appareils qui disparaissent.
Comme Home Assistant, le serveur fait maintenant une **sauvegarde chaque nuit**, toute seule,
**sans s'arrêter**.

## Ce qui se passe chaque nuit

1. À 3 h 30 (heure locale), le serveur copie sa base dans le dossier des sauvegardes :
   `ha-2026-10-15_033000.db`. La maison continue de fonctionner pendant la copie.
2. La copie est **vérifiée** : contrôle d'intégrité de SQLite, et présence des tables du serveur.
   Une copie ratée est supprimée tout de suite, jamais gardée comme « bonne ».
3. Seules les **7 dernières** sont gardées. Les fichiers qui ne portent pas ce format de nom ne
   sont jamais touchés.
4. Le résultat est écrit dans le journal d'activité, et visible dans le tableau de bord (tuile
   « Dernière sauvegarde » : « il y a 3 heures »).
5. **En cas d'échec** (disque plein…), la tuile l'affiche, et ton téléphone est prévenu si les
   notifications sont configurées (étape 12).

Si la machine était éteinte à 3 h 30, une sauvegarde est faite **2 minutes après le démarrage**
(dès que la dernière a plus d'un jour).

## Mettre en place

**Avec Docker** : rien à faire, c'est déjà actif. Les sauvegardes sont dans le volume
`ha-sauvegardes`. Pour les copier sur la machine (dans le dossier `sauvegardes/`), puis ailleurs
(clé USB, autre ordinateur…) :

```powershell
docker compose run --rm sauvegarde sh -c 'cp /automatiques/ha-*.db /sauvegardes/'
```

**Sans Docker** : indique le dossier.

```powershell
$env:HA_BACKUP_DIR = "D:\sauvegardes-maison"
$env:HA_BACKUP_TIME = "03:30"     # facultatif
$env:HA_BACKUP_KEEP = "7"         # facultatif, de 1 à 365
cargo run -p ha-server
```

## Les commandes

```powershell
cargo run -p ha-server -- backup now                 # une sauvegarde tout de suite (serveur en marche ou non)
cargo run -p ha-server -- backup list                # les sauvegardes du dossier
cargo run -p ha-server -- backup restore ha-2026-10-15_033000.db
```

Avec Docker : `docker compose run --rm ha-server backup list`, etc.

## Restaurer

1. **Arrête le serveur** (`docker compose stop ha-server`).
2. `backup restore <fichier>` :
   - vérifie la sauvegarde avant de toucher à quoi que ce soit ;
   - **met de côté la base actuelle** (`avant-restauration-<date>.db`, dans le dossier des
     sauvegardes, jamais supprimée par la rotation) : une restauration ratée se défait ;
   - remplace la base d'un coup (copie à côté, puis renommage) : jamais de base à moitié écrite.
3. Redémarre le serveur.

La commande **refuse** de restaurer si le serveur semble en marche :

- il répond à son adresse ;
- ou le journal `ha.db-wal` existe : SQLite le garde tant que la base est ouverte, et l'efface
  à l'arrêt propre. Cette vérification marche aussi entre deux conteneurs Docker, qui ne se voient
  pas l'un l'autre.

Après une **coupure de courant**, le journal reste. Si tu es sûr que le serveur est arrêté, ajoute
`--force`. Les dernières écritures du journal sont alors gardées dans la copie
`avant-restauration-…`.

## Ce qui est dans une sauvegarde, et ce qui n'y est pas

**Dedans** : comptes (mots de passe hachés avec Argon2id, jamais en clair), automatisations de
l'éditeur et leurs versions, historique, journal d'activité, associations Hue, Somfy et Freebox
(leurs jetons **chiffrés**).

**Pas dedans** :

- **la clé `ha.key`**, volontairement : une sauvegarde volée ne permet pas de piloter tes appareils.
  Mets la clé de côté **une fois pour toutes** dans ton gestionnaire de mots de passe (voir
  l'[étape 7](etape-7-deploiement.md)). Sans elle, une restauration fonctionne quand même : il faut
  seulement refaire `hue pair`, `somfy pair` et `freebox pair` ;
- tes fichiers `config/*.yaml` (automatisations, pièces, scripts…) : copie-les avec le reste, ce
  sont de simples fichiers.

Une sauvegarde contient **tout l'historique** de la maison (qui est là, quand). Elle est créée avec
des droits réservés à son propriétaire (600). Si tu la copies hors de chez toi (cloud), chiffre-la
avant (par exemple avec 7-Zip et un mot de passe).

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [backup.rs (stockage)](../crates/ha-storage/src/backup.rs) | Copie, vérification, rotation, restauration |
| [backup.rs (serveur)](../crates/ha-server/src/backup.rs) | Planification, entité, commandes `backup` |

**La copie à chaud.** On ne peut pas copier le fichier `ha.db` pendant que le serveur écrit : on
risquerait d'attraper une base à moitié modifiée. SQLite a une commande faite pour ça :
`VACUUM INTO 'fichier'`. Elle écrit une copie **cohérente** (comme figée à un instant précis) et
compactée, pendant que les autres connexions continuent de lire et d'écrire.

**Concepts Rust rencontrés :**

- **Même un chemin de fichier passe en paramètre SQL** : `VACUUM INTO ?1` avec `.bind(chemin)`,
  jamais collé dans la requête. Un nom de dossier contenant une apostrophe ne peut rien casser.
- **`#[cfg(unix)]`** : le code des droits de fichiers (600) n'est compilé que sous Linux et macOS.
  Sous Windows, une autre version de la même fonction ne fait rien. Le compilateur choisit selon le
  système.
- **Valider les noms** : l'horodatage d'un fichier ne peut contenir que des chiffres, `-` et `_`.
  Ainsi, aucun nom ne peut contenir `/` ou `..` et sortir du dossier.
- **`Option::is_none_or`** : « pas de sauvegarde du tout, **ou** la dernière a plus de 26 heures »,
  en une expression.

## Testé

- Sauvegarde pendant que la base est ouverte ; copie vérifiée ; droits 600 ; nom déjà pris refusé ;
  horodatages piégés (`../../etc`, `a/b`) refusés.
- Rotation : seules les 3 plus récentes gardées, fichiers étrangers et noms différents intacts.
- Restauration : l'ancienne valeur revient ; copie de sécurité saine, jamais supprimée par la
  rotation ; un faux fichier et une base SQLite étrangère sont refusés sans toucher à la base.
- **Serveur en marche** : `backup now` a fonctionné serveur allumé (140 Kio) ; la restauration a
  été refusée ; la sauvegarde de rattrapage a été faite 2 minutes après le démarrage, et la tuile
  « Dernière sauvegarde » a été mise à jour. Après l'arrêt propre, le journal `ha.db-wal` avait
  disparu ; la restauration a fonctionné, et le serveur a redémarré avec un journal d'activité
  intact.
- **Après une coupure brutale** (`kill -9`) : la restauration est refusée à cause du journal, puis
  acceptée avec `--force`.
- **Dans le conteneur Docker durci** (utilisateur sans droits, système en lecture seule) : sauvegarde
  dans le volume `ha-sauvegardes` ; restauration lancée depuis **un autre conteneur** pendant que
  le serveur tourne : refusée ; une fois le serveur arrêté : restaurée, et le serveur redémarre.
