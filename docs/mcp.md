# Étape 55 : brancher un assistant IA (MCP), expliqué

## Le but

Laisser un assistant IA, Claude par exemple, **lire et piloter la maison**, sans lui donner les
clés. C'est l'idée centrale de Moli OS (des agents qui pilotent la maison sans pouvoir nuire), et
Home Assistant propose la même chose avec son « serveur MCP ».

**MCP** (Model Context Protocol) est la façon standard de brancher des outils sur un assistant
IA. La maison en offre trois :

| Outil | Ce que l'assistant peut faire |
|-------|-------------------------------|
| `get_house_state` | lire l'état des appareils que son compte voit, rangés par pièce |
| `assist` | demander en français, comme dans [Assist](assist.md) : « ferme le volet de la chambre » |
| `call_service` | donner un ordre précis : `cover.set_cover_position` sur `cover.chambre`, position 40 |

## Les règles d'un assistant IA

Un assistant IA n'est **jamais** traité comme une personne, même si son compte est administrateur :

| Règle | Pourquoi |
|-------|----------|
| Dans une **pièce protégée** ou aux **heures calmes**, son ordre attend qu'un membre de la famille le valide (cloche, ou bouton « Valider » sur le téléphone) | comme un ordre à la voix : un humain décide |
| Il ne peut **jamais** déverrouiller une porte ni désarmer l'alarme | ouvrir la maison demande un mot de passe, qu'un assistant n'a pas |
| Il ne voit et ne commande que ce que **son compte** permet | un compte « invité » ne voit ni la présence ni la serrure |
| Ses valeurs sont **vérifiées** ([étape 53](ordres.md#étape-53--vérifié-avant-lenvoi)) | « consigne entre 15 et 25 (reçu 30) » |
| **60 appels** par minute au plus | un assistant qui boucle ne fait pas claquer les volets |
| Tout est écrit dans le **journal d'audit**, et le journal de bord dit « par l'assistant IA « claude » » | on sait toujours qui a fait quoi |

## Mettre en place

1. **Un compte pour l'assistant**, séparé du tien. « Invité » s'il doit seulement allumer et
   éteindre (lumières, prises, volets), sinon « famille » :

   ```powershell
   ha-server user add claude invite
   ```

   Le mot de passe demandé ne servira pas : l'assistant utilisera un jeton.

2. **Son jeton**, valable 90 jours (de 1 à 365) :

   ```powershell
   ha-server user token claude 90
   ```

   Il n'est affiché qu'**une seule fois**. Garde-le comme un mot de passe.

3. **Le brancher sur l'assistant.** L'adresse est celle du tableau de bord, suivie de `/api/mcp`.
   Avec Claude Code, par exemple :

   ```powershell
   claude mcp add --transport http maison https://maison.ton-reseau.ts.net/api/mcp --header "Authorization: Bearer <jeton>"
   ```

   Tout client MCP qui sait parler en « Streamable HTTP » avec un en-tête `Authorization`
   convient.

L'assistant doit pouvoir **joindre** le serveur : il tourne donc sur un appareil de ta maison ou
de ton réseau privé (Tailscale, [étape 7](etape-7-deploiement.md)). N'ouvre pas de port sur
Internet pour lui.

**Révoquer** l'assistant : `ha-server user passwd claude` ferme toutes ses sessions, jeton
compris ; `ha-server user delete claude` supprime le compte.

## À savoir avant de le brancher

- **Ce que l'assistant lit part chez son fournisseur.** L'état de la maison (qui est là, quelles
  portes sont ouvertes, les températures) est envoyé au service d'IA (Anthropic pour Claude) pour
  qu'il puisse répondre. Le compte « invité » limite ce qu'il voit.
- Un assistant IA peut se tromper. La garde et les vérifications l'empêchent de faire quelque
  chose de grave, pas de faire quelque chose de bête (éteindre la lumière du salon pendant que tu
  lis). Commence avec un compte invité.

## Pour les développeurs

`POST /api/mcp` : JSON-RPC 2.0, transport « Streamable HTTP », réponses en JSON (pas de flux SSE,
pas de session). Versions du protocole : `2025-06-18`, `2025-03-26`, `2024-11-05`.
`GET /api/mcp` répond 405 (le serveur ne pousse rien). Une requête qui annonce l'origine d'un
autre site (en-tête `Origin`) est refusée, comme le demande la spécification MCP : une page web
ne peut pas s'en servir.

## Comment c'est construit

| Fichier | Rôle |
|---------|------|
| [ha-api/src/mcp.rs](../crates/ha-api/src/mcp.rs) (nouveau) | Le protocole, les trois outils, la limite d'appels |
| [ha-api/src/rest.rs](../crates/ha-api/src/rest.rs) | `execute` : un ordre d'assistant IA est retenu par la garde quel que soit son rôle, et ne peut pas ouvrir la maison |
| [ha-core/src/origin.rs](../crates/ha-core/src/origin.rs) | `Origin::Agent` : « par l'assistant IA « claude » » |
| [ha-auth/src/lib.rs](../crates/ha-auth/src/lib.rs) | `issue_program_token` : un jeton sans mot de passe, créé en ligne de commande |
