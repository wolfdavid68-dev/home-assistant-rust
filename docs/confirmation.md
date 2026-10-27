# Étape 95 : la confirmation après un ordre, expliquée

## Le but

Quand Assist répond « C'est fait », l'appareil **a vraiment obéi**. Avant, « C'est fait » voulait
seulement dire « l'ordre est parti » : une lampe éteinte au mur ou un volet hors de portée
donnaient la même réponse qu'un ordre réussi.

Maintenant, après l'envoi, Assist attend **au plus 3 secondes** que chaque appareil visé change
d'état, puis répond :

| Ce qui s'est passé | La réponse |
|--------------------|------------|
| L'appareil a obéi | « C'est fait : fermer Volet chambre. » |
| Pas de nouvelle au bout de 3 s | « J'ai envoyé l'ordre, mais Volet chambre n'a pas encore répondu. » |
| L'appareil est indisponible | « J'ai envoyé l'ordre, mais Lampe cuisine est indisponible. » (`response_type` : `error`) |

Plusieurs appareils : « Volet chambre et Volet salon n'ont pas encore répondu », « … sont
indisponibles ». Les appareils qui ont obéi restent dans « C'est fait : … ».

C'est la même chose quel que soit le chemin de la phrase : la fenêtre Assist, la voix
([étape 84](voix.md)) ou un assistant IA par MCP ([étape 55](mcp.md)).

## Rester rapide

Le but est une réponse **1,8 seconde** après la fin de la phrase. Donc :

- **aucune attente** quand l'appareil est déjà dans le bon état (« allume la lampe » alors
  qu'elle est allumée) ;
- la réponse part **dès que tout est confirmé**, sans attendre la fin des 3 secondes ;
- on **écoute** les changements d'état (le bus d'événements du serveur), on ne relit pas l'état
  en boucle ;
- on n'attend que les appareils dont l'état réel arrive vite (voir le tableau plus bas).

## Ce qui compte comme « obéi »

| Ordre | État attendu |
|-------|--------------|
| allumer, éteindre (lumière, prise, ventilateur) | `on`, `off` ; basculer : l'inverse de l'état d'avant |
| régler la luminosité | `brightness` à ±4 près (l'appareil arrondit) |
| ouvrir, fermer un volet, ou le mettre à 40 % | la position à ±2 près, ou le volet en route : « en train de s'ouvrir / se fermer », ou une position qui a bougé dans le bon sens (certains volets disent « ouvert » à 30 % pendant leur course) ; sans position connue, `open` ou `closed` |
| verrouiller, déverrouiller | l'état final, ou « en cours » |
| lecture, pause, stop, volume | `playing`, `paused`, `idle`, `volume_level` à ±0,02 |
| chauffage : mode, consigne, allumer, éteindre | le mode, la consigne, autre chose que `off`, `off` |

Un volet qui **commence** à bouger a obéi : on n'attend pas les 20 secondes de sa course. Un
ordre sans état attendu clair (arrêter un volet, choisir une source…) n'est pas attendu.

## Intégration par intégration

On a regardé, pour chaque intégration, comment l'état est mis à jour après un ordre. Deux règles :
**notre propre écriture n'est jamais une confirmation**, et une intégration qui ne peut pas
confirmer en 3 secondes n'est **pas** déclarée en retard à tort.

| Intégration | Après un ordre | Ce qu'Assist fait |
|-------------|----------------|-------------------|
| **Hue** | Le pont pousse le nouvel état dans son flux d'événements, en 0,1 à 0,5 s. Le serveur n'écrit rien d'avance. | Attend l'état réel |
| **Sonos** | L'enceinte est relue aussitôt après l'ordre, en 0,1 à 0,5 s. | Attend l'état réel |
| **MQTT avec un sujet d'état** (Zigbee2MQTT, Shelly, Tasmota…) | L'appareil publie son nouvel état, en 0,05 à 0,5 s. | Attend l'état réel |
| **MQTT sans sujet d'état** | Le serveur **suppose** que l'appareil a obéi et écrit l'état lui-même (mode « optimiste », comme Home Assistant). Cet état porte maintenant l'attribut `assumed_state: true`. | N'attend pas : ce serait se confirmer soi-même |
| **Somfy** (volets) | La box répond qu'elle a accepté l'ordre ; le volet met 15 à 30 s et la box ne dit pas « en mouvement ». | Garde la réponse de la box (l'ordre est refusé sinon), n'attend pas |
| **Chauffage Somfy** | Relu toutes les 5 s, la première fois 5 s après l'ordre. | Garde la réponse de la box, n'attend pas |
| **SmartThings** | Le nuage répond qu'il a accepté l'ordre ; l'appareil est relu 2 s puis 10 s après. Trop juste pour 3 s. | Garde la réponse du nuage, n'attend pas |
| **Appareils du serveur** (entrées, thermostat virtuel, scènes, scripts, groupes, délestage, simulation de présence, protection solaire) | L'état est écrit au moment de l'ordre : c'est l'état réel. | N'attend pas |

Dans tous les cas, un appareil **indisponible** est signalé.

Pour Somfy, le chauffage Somfy et SmartThings, l'ordre attendait déjà la réponse de la
passerelle (les ordres avec réponse, [étape 46](ordres.md)) : une passerelle qui
refuse donne « Pas fait : … ». C'est la meilleure confirmation possible en moins de 3 secondes.

## Pour les développeurs

- `confirm::watch` est appelé **avant** l'envoi (pour savoir ce que « basculer » veut dire) ;
  l'abonnement au bus est pris avant le premier ordre, pour ne manquer aucun changement.
- `confirm::wait` juge d'abord l'état actuel, puis écoute les `StateChanged` jusqu'à l'échéance.
  Si des événements sont perdus (bus saturé), il relit l'état des appareils encore attendus.
- La liste des intégrations rapides est `QUICK` dans `confirm.rs` : une nouvelle intégration n'est
  attendue que si on l'y ajoute, après avoir vérifié qu'elle n'écrit pas l'état d'avance.

## Comment c'est construit

| Fichier | Rôle |
|---------|------|
| [ha-api/src/confirm.rs](../crates/ha-api/src/confirm.rs) (nouveau) | L'état attendu, l'attente sur le bus, la phrase |
| [ha-api/src/assist.rs](../crates/ha-api/src/assist.rs) | `converse` : surveiller, envoyer, attendre, répondre |
| [ha-mqtt/src/bridge.rs](../crates/ha-mqtt/src/bridge.rs) | L'attribut `assumed_state` sur les appareils sans sujet d'état |

Tests : confirmé tout de suite, confirmé après un moment, jamais confirmé, déjà dans le bon
état, indisponible, et les intégrations lentes ou supposées non attendues (`confirm.rs`) ; la
même chose de bout en bout par `/api/conversation/process` (`tests/api/assist.rs`).

Limites : les ordres donnés en touchant le tableau de bord ne sont pas concernés (la carte montre
déjà l'état réel quand il arrive). Le seuil de 3 s est fixe.
