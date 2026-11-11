# Étapes 105 et 106 : le cerveau, Claude quand Assist ne comprend pas

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
| les **trois derniers échanges** de la même personne, s'ils datent de moins de 5 minutes | ce que dit un invité, ou un mot seul (« euh ») |
| les **préférences de la maison** (le texte écrit par la famille) | |
| pour « Pourquoi ? », les lignes du journal de bord dans la demi-heure autour | |
| pour le résumé du matin, la météo, l'agenda du jour, les colis et l'énergie d'hier | |

La phrase n'est écrite dans aucun journal : on y lit seulement qui a demandé et le temps de
réponse.

## Quand la phrase part à Claude

Seulement si **tout** est vrai :

- Assist n'a pas compris : pas d'ordre reconnu, aucun appareil trouvé, ou une demande qu'il ne
  sait pas faire (une phrase **ambiguë**, « la lampe ou la prise ? », reste chez Assist, qui
  demande laquelle) ;
- la personne a un compte **famille** ou **administrateur** (jamais un invité) ;
- la phrase a **deux mots** au moins, et 300 caractères au plus ;
- moins de **40 phrases** sont parties dans l'heure, et moins de **150** sur les dernières
  24 heures (`HA_CERVEAU_PAR_JOUR`, de 1 à 1000).

Les deux limites comptent tout ce qui part à Claude : les phrases, les « Pourquoi ? » et le
résumé du matin.

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

## La suite d'une conversation

Le hub se souvient des **trois derniers échanges** de chaque personne pendant **5 minutes**.
Claude peut donc suivre :

- « Raconte-moi une blague sur le jardin » ;
- « Et une autre sur la cuisine » : Claude sait de quoi on parle.

Après 5 minutes sans phrase pour Claude, ou au redémarrage du hub, la conversation repart de
zéro. Ce souvenir reste en mémoire : il n'est écrit nulle part.

## Les préférences de la maison

Dans **Mon compte**, la case « Préférences de la maison » garde un texte libre que Claude lit
avant chaque réponse. Par exemple :

> Cosy, c'est les lampes du salon à 30 %. Les enfants se couchent à 20 h 30.

Le texte est le même pour toute la famille : 2000 caractères au plus, sans caractère de
contrôle. Un compte famille ou administrateur peut le lire et le changer ; un invité ne le voit
pas. Sous la case, on lit qui l'a changé en dernier, et combien de questions sont parties à
Claude en 24 heures.

## « Pourquoi ? » dans le journal de bord

Quand le cerveau est branché, chaque ligne du **journal de bord** a un bouton « Pourquoi ? ».
Claude lit la ligne et ce qui s'est passé dans la demi-heure autour (60 lignes au plus), puis
explique en une ou deux phrases : « Le volet s'est fermé parce que le soleil se couchait : c'est
l'automatisation du soir. » La réponse s'affiche sous la ligne.

Seuls les comptes famille et administrateur voient le bouton.

## Le résumé du matin

Avec `HA_CERVEAU_MATIN` (l'heure, `07:15` par exemple), Claude écrit chaque jour à cette heure
un résumé en trois ou quatre phrases : la météo du jour, l'agenda du jour, les colis attendus et
l'énergie consommée hier. Ce qui manque est passé.

Le résumé arrive :

- sur les téléphones, par `notify.appli` ;
- sur la tablette, dans la notification « Résumé du matin », qui remplace celle de la veille.

Sans `HA_CERVEAU_MATIN`, pas de résumé. Si Claude ne répond pas, le hub l'écrit dans son
journal et réessaie le lendemain.

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

Deux réglages de plus, dans `environment` de ha-server :

- `HA_CERVEAU_PAR_JOUR: 150` : les questions à Claude sur 24 heures (de 1 à 1000) ;
- `HA_CERVEAU_MATIN: "07:15"` : l'heure du résumé du matin (heure de la maison).

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
| [crates/ha-api/src/cerveau_api.rs](../crates/ha-api/src/cerveau_api.rs) | les préférences, « Pourquoi ? » et le résumé du matin |
| [crates/ha-storage/src/cerveau.rs](../crates/ha-storage/src/cerveau.rs) | les préférences gardées dans la base |
