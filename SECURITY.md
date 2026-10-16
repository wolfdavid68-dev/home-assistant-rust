# Sécurité

Ce logiciel contrôle des appareils physiques (serrures, chauffage, prises…). Une faille peut avoir
des conséquences réelles, donc ces règles ne sont pas optionnelles.

## Code

- `#![forbid(unsafe_code)]` dans toutes les crates : pas de code `unsafe`.
- Pas de `unwrap()` / `expect()` sur des données venant de l'extérieur (réseau, fichiers, appareils).
- Toutes les entrées sont désérialisées dans des types stricts (`serde`, `deny_unknown_fields`) et validées.
- Requêtes SQL uniquement paramétrées (`sqlx`), jamais de concaténation de chaînes.
- Les logs ne contiennent jamais de mot de passe, de jeton ou de secret.
- Journal d'audit (connexions, échecs, commandes, gestion des comptes et des appareils), signé et chaîné
  (HMAC-SHA256) : une modification de la base se détecte (`ha-server audit verify`, et à chaque démarrage).

## Authentification

- Mots de passe hachés avec **Argon2id**, jamais stockés en clair.
- Jetons d'accès aléatoires (256 bits, générateur cryptographique), stockés **hachés**, comparés en temps constant.
- Limitation des tentatives de connexion (blocage temporaire après plusieurs échecs).
- Double authentification facultative par code à usage unique (TOTP, RFC 6238) : secret chiffré, code
  non rejouable, activable et désactivable uniquement sur la machine (`ha-server user totp`).
- Aucune route de l'API accessible sans authentification, sauf la page de connexion.
- Modifier les automatisations depuis le tableau de bord demande de **confirmer son identité** (mot de passe,
  et code si la double authentification est activée), valable 15 minutes pour ce jeton seulement ; chaque
  modification est journalisée et notifiée. `HA_AUTOMATION_EDITOR=off` met l'éditeur en lecture seule.
- Les **modèles** des automatisations (`{{ … }}`) ne sont pas du Jinja : un petit interpréteur qui ne sait
  que **lire** un état ou un attribut et appliquer quelques filtres. Pas de boucles, de calculs, de fichiers
  ni de réseau ; vérifiés au chargement, 2 000 caractères et 20 expressions au plus, résultat tronqué.
- Le **schéma** des automatisations n'est qu'une vue : ce qui est enregistré reste la configuration
  vérifiée par le serveur, avec la même confirmation d'identité. La place des blocs, rangée à part, s'enregistre
  sans confirmation (elle ne change rien à ce que fait l'automatisation) mais seulement avec le droit de modifier
  les automatisations ; elle est vérifiée (400 blocs, coordonnées bornées) et rien d'autre n'est gardé.
- **Rôles** (administrateur, famille, invité) vérifiés **par le serveur** à chaque demande, WebSocket
  compris ; tout droit non accordé explicitement est refusé. Un invité ne voit pas la présence de la
  famille ni l'alarme, et ne commande jamais un portail, une porte de garage ou une serrure. Les
  interrupteurs du serveur lui-même (simulation de présence, qui dit quand la maison est vide, et
  délestage) lui sont cachés, comme les rappels insistants (« Garage ouvert »). Il reste
  toujours au moins un administrateur ; les rôles ne se changent que sur la machine (`ha-server user role`).

## Réseau

- Par défaut, le serveur écoute seulement en local (`127.0.0.1`). L'ouverture au réseau local est un choix explicite.
- **Ne jamais exposer le serveur directement sur Internet.** Accès à distance uniquement via VPN (WireGuard, Tailscale).
- HTTPS à distance avec `tailscale serve` (vrai certificat) ; jamais `tailscale funnel`, qui publie sur Internet.
- MQTT : broker avec identifiants, et TLS dès qu'il sort de la machine.
  **Exception temporaire** : TLS pas encore disponible (la partie TLS de `rumqttc` 0.25 dépend de crates
  vulnérables, refusées par `cargo deny`). D'ici là, broker sur la même machine ou sur le réseau local
  uniquement, jamais joignable depuis Internet.

- Pont Philips Hue : HTTPS avec certificat **épinglé** à l'association (confiance au premier usage),
  jamais de vérification TLS désactivée.
- Notifications (ntfy) : HTTPS obligatoire hors réseau local, aucune redirection suivie, jeton lu dans un
  fichier et jamais journalisé, contenu jamais journalisé, au plus 10 notifications en 10 minutes.
- Notifications du navigateur (Web Push) : envoi uniquement aux services de notifications des
  navigateurs (liste fermée, HTTPS, port standard, sans redirection), contenu chiffré de bout en bout
  (RFC 8291), clé VAPID et secrets des appareils chiffrés dans la base ; famille et administrateur seulement.
  Boutons des notifications : ticket aléatoire de 256 bits, à usage unique, pour un seul message, un seul
  appareil et un seul compte, gardé en empreinte et en mémoire ; abonnement et rôle revérifiés au toucher.
- Assist (phrases en français) : compris sur le serveur, sans cloud ; seulement les appareils visibles du
  compte ; chaque ordre suit le chemin d'un toucher (droits, garde, mot de passe, valeurs) ; phrase jamais journalisée.
- Assistant IA (MCP) : jeton par assistant (ligne de commande, 1 à 365 jours, révoqué avec le mot de passe) ;
  ce jeton ne marche que pour MCP (refusé par le tableau de bord, l'API et le WebSocket) ; ses ordres sont
  retenus par la garde quel que soit son rôle ; jamais serrure, alarme, sirène, garage, portail ni porte ;
  60 appels par minute ; en-tête `Origin` d'un autre site refusé ; chaque appel audité.
- Agendas en ligne (ICS) : HTTPS seulement, sans redirection, 2 Mio au plus ; l'adresse (qui peut contenir une
  clé secrète) n'est jamais journalisée ni publiée, seulement le nom du site ; lecture seule.
- Alexa (faux pont Hue, sans authentification possible) : désactivée par défaut ; seuls les appareils listés
  dans `alexa.yaml` sont exposés, jamais serrure, alarme, portail ni porte de garage ; seules les adresses
  autorisées (les Echo, ou à défaut le réseau local) sont servies ; serveur séparé de l'API ; chaque ordre est
  journalisé ; port 80 jamais redirigé depuis la box.
- Thermostat : radiateur coupé immédiatement si la température est inconnue, la fenêtre ouverte ou la
  température maximale atteinte ; consignes bornées (5 à 30 °C) ; arrêté au premier démarrage ; un radiateur
  ne peut être commandé que par un seul thermostat ; pas de rafale d'ordres vers un appareil injoignable.
- Météo (Open-Meteo) : désactivée par défaut ; HTTPS obligatoire hors réseau local ; la position envoyée est
  arrondie à 0,1° (environ 10 km) et n'apparaît jamais dans les logs, même dans un message d'erreur ; réponses
  limitées à 256 Kio et valeurs vérifiées.
- Vigilance (Météo-France) : désactivée par défaut ; HTTPS obligatoire hors réseau local, aucune redirection
  suivie ; clé lue dans un fichier, envoyée seulement dans l'en-tête `apikey`, absente des logs, des erreurs et
  de `Debug` ; la carte demandée est nationale (le département n'est pas envoyé) ; réponses limitées à 4 Mio,
  valeurs vérifiées, département absent ou carte périmée jamais pris pour du vert.
- Freebox : même principe, en **lecture seule** ; le jeton ne circule jamais (session par défi HMAC).
- Box Somfy TaHoma : même principe. Le compte Somfy ne sert qu'une fois, à créer le jeton local ; le mot de
  passe n'est jamais enregistré. Portails et portes de garage ne sont pas pilotables sans réflexion dédiée.

## Interface web

- Politique de sécurité du contenu (CSP) stricte : aucun script dans la page, aucune ressource extérieure.
- Données venues du réseau insérées uniquement avec `textContent`, jamais `innerHTML` (vérifié par un test).
- Dessin de la maison (style « plan ») : seules des images PNG, JPEG ou WebP, reconnues à leur contenu (jamais
  de SVG), 3 Mio et 16 images au plus ; modifiable par l'administrateur seulement, journalisé ; un invité n'en
  voit pas les calques ni les valeurs des appareils qui lui sont cachés. `img-src` accepte `blob:` : les images,
  lues avec le jeton, sont affichées depuis la mémoire de la page.

## Secrets

- Aucun secret dans le dépôt Git. Ils vont dans `secrets.yaml` ou `.env`, ignorés par Git.
- Les secrets que le serveur doit relire (clé du pont Hue, jeton Somfy) sont **chiffrés** dans la base
  (XChaCha20-Poly1305), avec une clé rangée dans un fichier séparé (`ha.key`, droits 600). Une copie
  de la base seule ne permet pas de piloter les appareils.
- Sauvegardes automatiques : fichiers en droits 600, vérifiées après copie, sans la clé `ha.key` (gardée à
  part) ; restauration refusée si le serveur semble en marche, avec copie de sécurité de la base actuelle.
- Si un secret est poussé par erreur, on le **change** immédiatement (le supprimer de Git ne suffit pas).

## Dépendances

- Le moins de dépendances possible, et des crates connues et maintenues.
- `cargo audit` et `cargo deny` dans la CI : la CI échoue si une dépendance a une faille connue.
- Dependabot activé pour les mises à jour.
- `Cargo.lock` versionné.

## Processus

- Tout changement passe par une pull request ; la CI (fmt, clippy, tests, audit) doit être verte avant de fusionner.
- En Docker : utilisateur non-root, système de fichiers en lecture seule, aucune capacité Linux, image sans shell
  épinglée par empreinte, port publié sur `127.0.0.1` par défaut (voir `compose.yaml`).

## Signaler une faille

Dépôt privé : ouvrir une issue avec l'étiquette `security`.
