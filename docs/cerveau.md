# Étape 105 : le cerveau, Claude quand Assist ne comprend pas

## Le but

[Assist](assist.md) comprend des phrases simples (« allume la cuisine », « quelle température au
salon ? ») sans rien envoyer hors de la maison. Quand il ne comprend pas, il répond « Je n'ai pas
compris ». Le **cerveau** prend le relais : la phrase est passée à **Claude**, qui répond, et
peut agir sur la maison avec les mêmes outils qu'un [assistant IA branché par MCP](mcp.md).

- « Raconte-moi une blague sur le jardin » : Claude répond.
- « Est-ce que je dois arroser ce soir ? » : Claude lit l'état de la maison (pluie, humidité…)
  et répond.
- « Mets une ambiance cosy au salon » : Claude baisse les lumières du salon, si son compte le
  permet.

Ça marche partout où Assist répond : la fenêtre Assist, le micro de la tablette, le mot d'appel
« Maison ». La réponse est lue à voix haute comme les autres.

Le cerveau utilise **Claude Code avec ton abonnement Claude** (Pro ou Max), pas une clé d'API
payante. Il est **facultatif** : sans lui, rien ne change.

## Ce qui sort de la maison

| Sort | Ne sort jamais |
|------|----------------|
| le **texte** des phrases qu'Assist n'a pas comprises | le son : la voix est toujours écoutée sur cette machine |
| ce que Claude lit par les outils de la maison pour répondre (l'état des appareils que son compte voit) | les phrases qu'Assist comprend lui-même |
| | ce que dit un invité, ou un mot seul (« euh ») |

La phrase n'est écrite dans aucun journal : on y lit seulement qui a demandé et le temps de
réponse.

## Quand la phrase part à Claude

Seulement si **tout** est vrai :

- Assist n'a pas compris : pas d'ordre reconnu, aucun appareil trouvé, ou une demande qu'il ne
  sait pas faire (une phrase **ambiguë**, « la lampe ou la prise ? », reste chez Assist, qui
  demande laquelle) ;
- la personne a un compte **famille** ou **administrateur** (jamais un invité) ;
- la phrase a **deux mots** au moins, et 300 caractères au plus ;
- moins de **40 phrases** sont parties dans l'heure.

Sinon, Assist répond comme avant. Claude a **30 secondes** pour répondre ; une réponse prend
d'habitude de 5 à 15 secondes. Une seule phrase à la fois : pendant qu'il réfléchit, une autre
phrase reçoit la réponse habituelle d'Assist.

Chaque phrase compte dans les **limites de ton abonnement**, comme une question posée dans
l'appli Claude.

## Ce que Claude peut faire

Claude agit **par le serveur MCP** de la maison, avec un compte à lui, et rien d'autre : ni
fichiers, ni commandes, ni web. Toutes les [règles d'un assistant IA](mcp.md#les-règles-dun-assistant-ia)
s'appliquent :

- il ne commande **jamais** une serrure, l'alarme, une sirène, une porte de garage, un portail
  ou une porte ;
- dans une pièce protégée ou aux heures calmes, son ordre attend qu'un membre de la famille le
  valide ;
- avec un compte **invité** (conseillé), il ne voit ni la présence ni la serrure ;
- tout est écrit dans le journal d'audit.

## Mettre en place

Le cerveau est un service Docker de plus, à côté de ha-server. Les trois secrets vont dans le
dossier `secrets/`, **hors du dépôt**.

1. **Le jeton de ton abonnement.** Sur un ordinateur où Claude Code est installé, lance :

   ```powershell
   claude setup-token
   ```

   Connecte-toi avec ton compte Claude quand le navigateur s'ouvre. Le jeton affiché (il
   commence par `sk-ant-oat`) vaut environ **un an**. Copie-le, puis :

   ```powershell
   Get-Clipboard | Set-Content -NoNewline secrets\claude_oauth
   ```

   Garde-le comme un mot de passe : il permet de parler à Claude avec ton abonnement.

2. **La clé entre ha-server et le cerveau**, tirée au hasard :

   ```sh
   openssl rand -hex 32 > secrets/cerveau_cle
   ```

   Sous Windows, sans openssl :

   ```powershell
   $b = New-Object byte[] 32; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b)
   -join ($b | ForEach-Object { $_.ToString('x2') }) | Set-Content -NoNewline secrets\cerveau_cle
   ```

3. **Le compte de Claude dans la maison**, et son jeton (voir [MCP](mcp.md#mettre-en-place)) :

   ```sh
   docker compose run --rm ha-server user add cerveau invite
   docker compose run --rm ha-server user token cerveau 90
   ```

   Mets le jeton affiché dans `secrets/cerveau_maison`. Tu peux aussi reprendre le compte
   « claude » de l'étape 55, avec un jeton à lui.

4. **Dans [compose.yaml](../compose.yaml)**, enlève les « # » :
   - du service `cerveau` ;
   - de `HA_CERVEAU`, `HA_CERVEAU_CLE_FILE` et du secret `cerveau_cle` dans ha-server ;
   - des secrets `cerveau_cle`, `claude_oauth` et `cerveau_maison` tout en bas.

5. **Sur le mini-PC (Linux)**, les deux services tournent sous l'utilisateur 65532 :

   ```sh
   sudo chown 65532:65532 secrets/cerveau_cle secrets/claude_oauth secrets/cerveau_maison
   sudo chmod 600 secrets/cerveau_cle secrets/claude_oauth secrets/cerveau_maison
   ```

6. **Construis et démarre** :

   ```sh
   docker compose build cerveau
   docker compose up -d
   ```

   Le journal de ha-server dit « cerveau : branché (les phrases non comprises lui sont
   passées) ». Essaie dans Assist : « Raconte-moi une blague sur la maison ».

Pour un modèle plus juste mais plus lent, ajoute `CERVEAU_MODELE: sonnet` au service `cerveau`
(Haiku si rien n'est dit).

## Les limites

- **Les jetons expirent** : celui de l'abonnement après un an environ, celui de la maison après
  le nombre de jours choisi. Quand l'un expire, le cerveau ne répond plus et Assist reprend ses
  réponses habituelles ; refais l'étape 1 ou 3, puis `docker compose restart cerveau`.
- **Pas avec `HA_TLS=on`** pour l'instant : le cerveau parle à ha-server en HTTP sur le réseau
  Docker.
- **Pas avec `network_mode: host`** (Alexa) : ha-server ne joindrait plus le cerveau par son
  nom.
- Le cerveau a besoin d'**Internet** (Claude) : il est sur le réseau Docker par défaut, sans port
  publié. Seul ha-server lui parle, avec la clé de l'étape 2.

## Les fichiers

| Fichier | Rôle |
|---------|------|
| [packaging/docker/cerveau/serveur.mjs](../packaging/docker/cerveau/serveur.mjs) | le service : reçoit la phrase, lance Claude Code avec les seuls outils de la maison, rend une réponse courte à lire |
| [packaging/docker/cerveau/Dockerfile](../packaging/docker/cerveau/Dockerfile) | l'image : Claude Code à une version fixée, vérifié par son empreinte, sans privilège |
| [crates/ha-api/src/cerveau.rs](../crates/ha-api/src/cerveau.rs) | côté ha-server : qui a droit au cerveau, 40 phrases par heure, la clé, le délai |
| [crates/ha-api/src/assist.rs](../crates/ha-api/src/assist.rs) | Assist passe au cerveau ce qu'il ne comprend pas |
