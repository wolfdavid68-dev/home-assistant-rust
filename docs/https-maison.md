# Étape 79 : le HTTPS sur le réseau de la maison, expliqué

## Le but

Jusqu'ici, le tableau de bord s'ouvrait en `http://192.168.1.20:8123` : sur le Wi-Fi, le mot de
passe et tout ce qui s'affiche circulaient **en clair**. N'importe quel appareil branché au réseau
(une prise connectée piratée, un invité) pouvait les lire.

Avec `HA_TLS=on`, tout passe en **HTTPS**, comme une banque en ligne : chiffré et vérifié. En plus,
le navigateur débloque ce qu'il réserve aux pages sécurisées :

- **garder l'écran allumé** (le mode tablette murale, sans appli en plus) ;
- les **notifications** sur le téléphone ;
- **installer le tableau de bord** comme une appli.

Sur le mini-PC, c'est activé d'office ([mini-PC](mini-pc.md)).

## Le principe : un certificat de la maison

Un site comme `impots.gouv.fr` prouve son identité avec un certificat signé par une autorité que
tous les appareils connaissent. Une adresse comme `192.168.1.20` n'en a pas : aucune autorité
publique ne signe une adresse privée.

La maison a donc **sa propre autorité** :

1. Au premier démarrage avec `HA_TLS=on`, le serveur crée l'**autorité de la maison**
   (`maison-ca.crt`, valable 10 ans). Elle est rangée à côté de la clé qui chiffre la base, à part
   des sauvegardes.
2. Elle signe le **certificat du serveur**, pour son adresse (`192.168.1.20`) et son nom
   (`maison.local`). Il est refait à chaque démarrage et chaque semaine : il suit l'adresse de la
   machine. Sa clé ne quitte jamais la mémoire.
3. **On installe l'autorité une fois sur chaque appareil** (téléphone, tablette, PC). Ensuite,
   ils font confiance au serveur : plus aucun avertissement.

### Une autorité qui ne peut rien signer d'autre

Installer une autorité sur un téléphone, c'est lui dire « crois ce qu'elle signe ». Si sa clé était
volée, quelqu'un pourrait-il se faire passer pour ta banque ? **Non.** L'autorité de la maison
porte des **contraintes de nom** : elle ne peut signer que

- des adresses privées : `192.168.x.x`, `10.x.x.x`, `172.16` à `172.31`, `127.x.x.x`, et leurs
  équivalents IPv6 ;
- des noms en `.local` (et `localhost`).

Un certificat pour `google.com` ou `8.8.8.8` signé avec sa clé est **refusé** par les appareils.
C'est vérifié : OpenSSL répond « permitted subtree violation », et Chromium refuse la page
(`ERR_CERT_INVALID`).

## 1. Activer

- **Mini-PC** : rien à faire, `HA_TLS=on` est dans `/etc/ha-maison/ha-maison.env`. Pour une
  installation plus ancienne, ajoute la ligne `HA_TLS=on`, puis
  `sudo systemctl restart ha-maison`.
- **Windows** : dans `demarrer.bat`, enlève le `rem` devant `set HA_TLS=on`.
- **Docker** : dans `compose.yaml`, `HA_TLS: "on"` et l'adresse de la machine dans
  `HA_TLS_NOMS` (un conteneur ne la voit pas).

Le serveur écrit dans son journal :

```text
HTTPS : https://192.168.1.20:8123/ (certificat pour localhost, 127.0.0.1, ::1, 192.168.1.20, maison.local)
certificat de la maison, à installer une fois sur chaque appareil : http://192.168.1.20:8123/maison-ca.crt (empreinte SHA-256 BA:B7:…)
```

`sudo ha-maison https` (Windows : `ha-server.exe https`) affiche la même chose à tout moment.

**Les anciens favoris** en `http://…:8123` marchent encore : ils sont renvoyés vers `https://`.

## 2. Installer le certificat de la maison sur chaque appareil

Sur l'appareil, ouvre **`http://192.168.1.20:8123/maison-ca.crt`** (avec l'adresse de ton
serveur, et bien en `http`). Le fichier `maison-ca.crt` se télécharge. Puis :

### Android (téléphone, tablette)

1. **Paramètres**, puis cherche « **certificat** » dans la barre de recherche des paramètres.
2. Ouvre **Installer un certificat** (ou « Installer depuis le stockage ») → **Certificat CA**.
   - Pixel : Sécurité et confidentialité → Plus de paramètres de sécurité → Chiffrement et
     identifiants → Installer un certificat → Certificat CA.
   - Samsung : Sécurité et confidentialité → Autres paramètres de sécurité → Installer depuis le
     stockage de l'appareil → Certificat CA.
3. **Installer quand même**, puis choisis `maison-ca.crt` dans les téléchargements.

Android affiche ensuite « Le réseau peut être surveillé ». C'est son message pour toute autorité
ajoutée à la main ; celle de la maison ne peut rien signer en dehors de la maison (plus haut).

### iPhone, iPad

1. Ouvre le lien dans **Safari** : « Profil téléchargé ».
2. **Réglages → Profil téléchargé → Installer**.
3. **Réglages → Général → Informations → Réglages des certificats** : active
   « **Maison (autorité locale)** ».

Les détails du profil montrent l'**empreinte SHA-256** : elle doit être la même que celle du
journal ou de `sudo ha-maison https`.

### Windows (Chrome, Edge)

1. Double-clique `maison-ca.crt` → **Installer un certificat** → **Utilisateur actuel**.
2. **Placer tous les certificats dans le magasin suivant** → **Parcourir** → **Autorités de
   certification racines de confiance**.
3. **Terminer**, puis **Oui**. Redémarre le navigateur.

**Firefox** a ses propres autorités : Paramètres → Vie privée et sécurité → Certificats →
**Afficher les certificats** → Autorités → **Importer**, puis coche « Confirmer cette AC pour
identifier des sites web ».

### Mac

Double-clique `maison-ca.crt` : il s'ouvre dans **Trousseaux d'accès**. Double-clique
« Maison (autorité locale) » → **Se fier** → **Toujours approuver**.

## 3. Ouvrir le tableau de bord

`https://192.168.1.20:8123` (ou `https://maison.local:8123` si l'appareil connaît les noms en
`.local`). Le cadenas s'affiche, sans avertissement. Mets à jour les favoris et l'écran
d'accueil.

**La tablette murale avec Fully Kiosk Browser** : mets l'adresse en `https://` dans *Start URL*,
après avoir installé le certificat sur la tablette. Pas encore essayé sur une vraie tablette : si
Fully refuse la page alors que Chrome l'ouvre, dis-le-moi.

## Si quelque chose cloche

- **« Votre connexion n'est pas privée »** : le certificat de la maison n'est pas installé sur cet
  appareil, ou pas comme **autorité** (CA). Reprends l'étape 2.
- **Le serveur a changé d'adresse** : le certificat suit au prochain démarrage (ou dans la
  semaine). Mieux : une adresse fixe réservée dans la box ([mini-PC](mini-pc.md)).
- **Un autre nom ou une autre adresse** : `HA_TLS_NOMS=maison.local,192.168.1.21` (seulement des
  noms en `.local` et des adresses privées).
- **Revenir en HTTP** : `HA_TLS=off`. L'autorité reste rangée, prête pour plus tard.
- **Recommencer de zéro** (nouvelle autorité) : arrête le serveur, supprime `maison-ca.crt` et
  `maison-ca.key` du dossier de la clé (`/var/lib/ha-maison-cle` sur le mini-PC), redémarre, et
  réinstalle le nouveau certificat sur chaque appareil (retire l'ancien).

## Sécurité

- **Chiffré et vérifié** : TLS 1.2 et 1.3 (rustls, en Rust), les suites de chiffrement sûres
  seulement.
- **L'autorité ne signe que la maison** (contraintes de nom, plus haut), et **ne signe pas d'autre
  autorité** (`pathlen:0`).
- **Sa clé** (`maison-ca.key`) est créée lisible par le seul compte du serveur (`0600`), à côté de
  la clé de la base, et n'est **jamais dans les sauvegardes**. La clé du certificat du serveur,
  elle, n'existe qu'en mémoire.
- **En clair, une seule chose** : le certificat de l'autorité (`/maison-ca.crt`), qui est public.
  Tout le reste est renvoyé vers `https://`. La redirection ne recopie que des adresses sages : pas
  d'en-tête glissé par un lien piégé.
- **Une limite** : le certificat se télécharge en clair, la première fois. Quelqu'un qui contrôle
  déjà ton réseau pourrait en glisser un autre à ce moment-là. Pour t'en assurer, compare
  l'empreinte (iPhone, Mac) avec `sudo ha-maison https`.
- **Les poignées de main** se font chacune à part, en 10 secondes au plus et 128 en même temps :
  un appareil lent ou malveillant ne bloque pas les autres.
- Les en-têtes de sécurité (CSP, `nosniff`…) sont les mêmes en HTTPS. Le port 8124 des enceintes
  Sonos et le port 80 d'Alexa restent en HTTP : ces appareils ne savent pas faire autrement, et ils
  ne transportent pas de mot de passe.

## Comment c'est construit

- **`crates/ha-tls/src/local_ca.rs`** (nouveau) :
  - `LocalCa::load_or_create` : lit l'autorité, ou la crée (clé en `0600`) ; refuse de remplacer
    une autorité à moitié présente ;
  - `issue_server` : un certificat de 397 jours pour les noms donnés, signé par l'autorité ;
  - `LocalName::parse` : n'accepte que ce que les contraintes de nom permettent ;
  - `ServerCert` : le certificat du moment, remplaçable sans redémarrer.
- **`crates/ha-api/src/tls.rs`** (nouveau) : `serve_https`. Le premier octet de chaque connexion
  dit si c'est du TLS (`0x16`) ou du HTTP en clair. Le TLS va à axum, le clair reçoit la
  redirection ou le certificat.
- **`crates/ha-server/src/https.rs`** (nouveau) : `HA_TLS`, `HA_TLS_NOMS`, `HA_TLS_DIR`, les noms
  du certificat (adresse de la machine, nom en `.local`), le renouvellement chaque semaine,
  `ha-server https`. `healthcheck` accepte la redirection : le serveur est bien là.
- **Mini-PC** : `HA_TLS=on` dans `ha-maison.env`, et l'installateur donne l'adresse `https://` et le
  lien du certificat.

**Tests** :
- les noms permis et refusés ;
- l'autorité créée une fois puis relue (même empreinte, clé en `0600`), jamais à moitié remplacée ;
- de bout en bout : un client qui ne connaît que l'autorité de la maison ouvre l'API en HTTPS (avec
  les en-têtes de sécurité), le HTTP en clair est redirigé, le certificat se télécharge, et un
  client qui ne connaît pas la maison est refusé ;
- la réponse en clair : redirection, certificat, rien d'injectable.

Essayé :
- **OpenSSL** : certificat valide pour `192.168.1.20` et `maison.local`, faux certificats
  `google.com` et `8.8.8.8` refusés ;
- **Chromium**, avec l'autorité installée : `http://` renvoyé vers `https://`, page sécurisée,
  « En direct » par le WebSocket chiffré, écran gardé allumé, service worker actif, `maison.local`
  accepté, faux `google.com` refusé, aucune erreur JavaScript ;
- **le mini-PC** (Debian 13, service protégé) : installation neuve en HTTPS, autorité créée dans
  `/var/lib/ha-maison-cle`, `curl` accepte le serveur avec le certificat de la maison et le refuse
  sans.

### Concepts utilisés

**Les contraintes de nom (RFC 5280)** : une extension du certificat d'une autorité, marquée
« critique », qui liste ce qu'elle a le droit de signer. Un appareil qui ne la comprend pas doit
refuser le certificat plutôt que l'ignorer.

**Rust : un `Listener` à soi pour axum** : axum sait servir n'importe quelle source de connexions
qui implémente son trait `Listener`. La nôtre reçoit d'un canal (`mpsc`) les connexions TLS déjà
prêtes, que des tâches à part ont acceptées.

**Rust : `RwLock<Arc<…>>`** : le certificat du moment, lu à chaque connexion et remplacé chaque
semaine, sans arrêter le serveur.
