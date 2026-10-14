# Étape 14 : Alexa en local, expliqué

## Le but

« Alexa, ferme le volet du salon. » « Alexa, allume la cafetière. » Piloter la maison à la voix,
**sans** skill Alexa, **sans** compte à relier, et **sans** ouvrir de port sur Internet.

## L'astuce : se faire passer pour un pont Philips Hue

Un Echo sait piloter un **pont Philips Hue** tout seul, directement sur le réseau local. Le
serveur se présente donc comme un pont Hue, comme le fait `emulated_hue` dans Home Assistant :

1. « Alexa, découvre mes appareils » : l'Echo envoie un message de **découverte** sur le réseau
   local (SSDP, port UDP 1900). Le serveur répond « je suis un pont, ma fiche est à telle adresse ».
2. L'Echo lit la fiche, puis la liste des « lampes » : ce sont tes appareils de `alexa.yaml`.
3. « Alexa, allume la cafetière » : l'Echo envoie l'ordre **directement** au serveur, en HTTP sur
   le réseau local (port 80). Le serveur allume la prise.

Pour Alexa, tout est une **lampe**. Le serveur traduit :

| Appareil | « allume » / « éteins » | « mets à 50 % » |
|----------|-------------------------|-----------------|
| Lampe | allume / éteint | luminosité |
| Prise | allume / éteint | — |
| Volet | **ouvre** / **ferme** | position |
| Script | lance / arrête | — |
| Scène | active | — |

**Ce qui passe par Internet, et ce qui n'y passe pas.** Ta voix est toujours comprise par les
serveurs d'Amazon : c'est le fonctionnement de tout Echo. Mais ni le serveur domotique ni tes
appareils ne parlent à Amazon. Aucun compte n'est relié, aucun port n'est ouvert sur ta box :
l'ordre va de l'Echo au serveur, dans ta maison.

## Mettre en place

### 1. Choisir les appareils

Copie [alexa.example.yaml](../alexa.example.yaml) en `alexa.yaml` (avec Docker :
`config/alexa.yaml`) et garde ce que tu veux piloter à la voix :

```yaml
- entity: switch.prise_cafe
  name: Cafetière
- entity: cover.volet_salon
  name: Volet du salon
```

`name` est le nom à **prononcer**. Choisis des noms courts et faciles à dire, tous différents.
Puis vérifie :

```powershell
cargo run -p ha-server -- alexa check
```

### 2. Indiquer l'adresse de la machine, et celle des Echo

```powershell
ipconfig                                      # « Adresse IPv4 » de cette machine
$env:HA_ALEXA_IP = "192.168.1.20"             # cette machine
$env:HA_ALEXA_ALLOWED = "192.168.1.40"        # tes Echo, séparés par des virgules
cargo run -p ha-server
```

L'adresse de chaque Echo se trouve dans l'appli Alexa (l'appareil, puis **Paramètres**,
**À propos**) ou dans l'interface de la Freebox (« Périphériques réseau »). Pour qu'elle ne change
pas, donne-lui une **adresse fixe** dans la Freebox (« Baux DHCP statiques »). Fais de même pour la
machine du serveur.

Au démarrage, le serveur affiche :

```
Alexa : pont Hue sur http://192.168.1.20:80/, 2 appareil(s) exposé(s)
```

### 3. Le pare-feu et le port 80

- Les Echo récents ne parlent à un pont Hue que sur le **port 80**. C'est la valeur par défaut
  (`HA_ALEXA_PORT` permet d'en changer, mais un Echo récent ne le suivra pas).
- Windows demande d'autoriser `ha-server` : coche **uniquement « Réseaux privés »**.
- Si le serveur affiche « Alexa désactivée : impossible d'écouter sur … :80 », un autre logiciel
  utilise déjà ce port (souvent IIS, « Services World Wide Web »). Arrête-le, ou installe le
  serveur sur une autre machine.
- Le reste de la maison continue de fonctionner, même si Alexa n'a pas pu démarrer.

### 4. Découvrir les appareils

Dis « **Alexa, découvre mes appareils** » (ou, dans l'appli Alexa : **Appareils**, **+**,
**Ajouter un appareil**, **Philips Hue**, puis lance la découverte). Tes appareils apparaissent
avec les noms de `alexa.yaml`. Tu peux ensuite les ranger dans des **groupes** Alexa
(« Salon »), ce qui permet de dire « Alexa, éteins le salon ».

Exemples :

- « Alexa, allume la cafetière » / « éteins la cafetière »
- « Alexa, ferme le volet du salon » / « mets le volet du salon à 30 % »
- « Alexa, allume la lampe du salon à 40 % »
- « Alexa, allume soirée cinéma » (une scène)

Pour **retirer** un appareil : enlève-le de `alexa.yaml`, redémarre, puis supprime-le dans l'appli
Alexa. S'il revient plus tard, il reprend le même numéro, et Alexa le reconnaît.

### Avec Docker

La découverte utilise le **multicast**, qui ne traverse pas le réseau isolé de Docker. Il faut
donc le réseau de la machine (`network_mode: host`), ce qui ne fonctionne que sous **Linux** :

1. Dans [compose.yaml](../compose.yaml), décommente `HA_ALEXA_IP`, `HA_ALEXA_ALLOWED` et
   `network_mode: host`. Avec le réseau de la machine, la ligne `ports` n'a plus d'effet :
   ajoute `HA_BIND: 192.168.1.20:8123` (le tableau de bord sur le réseau local) ou
   `HA_BIND: 127.0.0.1:8123` (cette machine seulement).
2. Le conteneur tourne sans aucun privilège : pour qu'il puisse écouter sur le port 80, autorise
   les ports à partir de 80 **sur la machine** :

   ```bash
   echo "net.ipv4.ip_unprivileged_port_start=80" | sudo tee /etc/sysctl.d/80-alexa.conf
   sudo sysctl --system
   ```

Sous Windows ou macOS (Docker Desktop), lance plutôt le serveur directement, sans Docker, pour
utiliser Alexa.

## La sécurité

Le langage d'un pont Hue n'a **pas de vraie authentification** : un Echo ne sait pas en faire.
Les protections sont donc ailleurs, et c'est pour ça qu'Alexa est désactivée par défaut :

- **Rien n'est exposé sans le dire.** Seuls les appareils de `alexa.yaml` existent pour Alexa.
  Les autres sont invisibles et impilotables, même en devinant leur numéro.
- **Jamais les appareils dangereux.** Serrures et alarmes sont refusées dans `alexa.yaml`, et un
  volet déclaré comme **portail ou porte de garage** est ignoré même s'il y figure. Ouvrir la
  maison à la voix, c'est permettre à n'importe qui criant « Alexa, ouvre le portail » devant une
  fenêtre de le faire.
- **Seules les adresses autorisées.** Avec `HA_ALEXA_ALLOWED`, seuls tes Echo sont servis. Sans
  cette variable, tout le réseau local l'est, mais jamais une adresse extérieure. Les autres
  reçoivent un refus, avant même la lecture de la requête.
- **Un serveur à part.** Le faux pont écoute sur son propre port, avec ses propres routes. Il ne
  donne accès ni à l'API, ni au tableau de bord, ni aux comptes, ni à l'historique.
- **Tout est tracé.** Chaque ordre d'Alexa va dans le journal d'activité signé (acteur
  « alexa », avec l'adresse de l'Echo), comme un ordre du tableau de bord.
- **Limites.** Requêtes de 4 Kio et 5 secondes au plus. La découverte ne répond pas plus de
  30 fois par 10 secondes : on ne peut pas se servir du serveur pour inonder une autre machine.
- **Jamais sur Internet.** Ne redirige **jamais** le port 80 depuis ta box. Rien dans cette
  étape ne le demande.

Limite honnête : sur le réseau local, une machine autorisée peut piloter les appareils de
`alexa.yaml` sans mot de passe, comme un vrai pont Hue. C'est pourquoi on n'y met que des
appareils sans danger, et qu'il vaut mieux indiquer l'adresse des Echo.

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [config.rs](../crates/ha-alexa/src/config.rs) | Lecture et vérification de `alexa.yaml` |
| [hue.rs](../crates/ha-alexa/src/hue.rs) | Traduction « lampe Hue » ↔ lampe, prise, volet, script, scène |
| [server.rs](../crates/ha-alexa/src/server.rs) | Le faux pont : routes HTTP, filtre d'adresses, journal |
| [ssdp.rs](../crates/ha-alexa/src/ssdp.rs) | La découverte (port UDP 1900) |
| [numbers.rs](../crates/ha-alexa/src/numbers.rs) | Numéros de lampes stables, gardés dans la base |

Deux détails qui comptent :

- **Des numéros qui ne changent jamais.** Alexa retient chaque appareil par son numéro de lampe.
  Le numéro est donné une fois pour toutes et gardé dans la base. Changer l'ordre de
  `alexa.yaml` ne mélange donc pas les appareils.
- **L'état promis.** Juste après un ordre, Alexa relit l'état de l'appareil. Un volet met
  plusieurs secondes à se fermer : sans précaution, Alexa dirait « l'appareil ne répond pas ».
  Pendant 5 secondes, le serveur annonce donc l'état demandé, comme Home Assistant.

**Concepts Rust rencontrés :**

- **Un middleware axum** (`middleware::from_fn_with_state`) : une fonction appelée **avant** toutes
  les routes. Le filtre d'adresses y est écrit une seule fois : impossible d'oublier une route.
- **UDP et multicast** avec `socket2` : on prépare la socket (partage du port 1900 avec les
  autres logiciels UPnP, abonnement au groupe 239.255.255.250), puis on la confie à tokio
  (`UdpSocket::from_std`).
- **`BTreeMap`** pour les lampes : triée par numéro, la liste envoyée à Alexa est toujours dans le
  même ordre.
- **`RETURNING` en SQL** : l'insertion renvoie directement le numéro attribué, sans seconde
  requête. Le tout dans une **transaction**, pour que deux numéros ne soient jamais donnés deux fois.
- **`IpAddr::to_canonical`** : une adresse IPv4 vue à travers IPv6 (`::ffff:192.168.1.40`)
  redevient `192.168.1.40`. Sans cela, le filtre d'adresses pourrait être contourné.

## Testé

- `alexa.yaml` : ordre, noms, refus des serrures, alarmes, capteurs, doublons, noms vides ou trop
  longs, fautes de frappe.
- Traduction : chaque type d'appareil, luminosité et position (aux extrêmes compris), état promis,
  ordres invalides refusés au format Hue.
- En jouant l'Echo : fiche UPnP, inscription, liste des lampes (sans le garage), ordres
  « allume », « mets à 30 % », « éteins », garage refusé même par son numéro, lampe inconnue,
  journal d'audit. Adresses extérieures refusées ; corps trop gros refusé.
- Numéros stables malgré un changement d'ordre, un retrait puis un retour.
- Découverte : réponses aux recherches des Echo, messages ignorés, vraie socket UDP.
- **Sur un serveur en marche** : un script jouant l'Echo a découvert le pont **en multicast**, lu
  les deux appareils et allumé la prise et la lampe (à 40 %). Une adresse non autorisée a reçu
  un refus.
- **Pas encore testé avec un vrai Echo** : le protocole est celui qu'utilise `emulated_hue` de
  Home Assistant depuis des années, mais Amazon peut changer le comportement de ses appareils. Si
  la découverte ne trouve rien, dis-le moi avec les logs du serveur.
