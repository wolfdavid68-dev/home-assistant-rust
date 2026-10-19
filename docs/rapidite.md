# Étape 63 : la rapidité des appareils, mesurée de bout en bout, expliqué

## Le but

L'[étape 47](performance.md) mesure le serveur : 95 % de ses réponses partent en moins de 25 ms.
Mais quand on touche une lampe, ce qu'on attend, c'est de **la voir s'allumer**. Entre le toucher
et ce moment, le temps se perd surtout hors du serveur : dans le pont Hue, la box Somfy, le réseau
Zigbee, l'appareil lui-même. Cette étape mesure ce temps-là, appareil par appareil. Elle accélère
ce qui dépend de nous (Somfy) et signale l'appareil qui traîne.

## Ce qui est mesuré

Pour chaque ordre (un toucher, une automatisation, Alexa, Assist…), le serveur note l'heure. Il
attend ensuite le **premier changement d'état** de l'appareil : la lampe passe à « allumée », le
volet à « en ouverture ». C'est le moment où le tableau de bord le montre.

- **Seuls les vrais appareils** sont mesurés : ceux d'une intégration (Hue, Somfy, MQTT). Une entrée
  virtuelle ou un état posé par l'API n'a pas d'appareil derrière lui.
- **Un ordre qui ne change rien** (allumer une lampe déjà allumée) n'a pas de réponse. Il est oublié
  au bout de 30 secondes, sans compter.
- **Un changement sans ordre** (la lampe allumée à l'interrupteur) ne compte pas non plus.
- Tout reste en mémoire : rien n'est écrit dans la base.

## Voir les chiffres

**Mon compte → Système** (administrateur) ajoute une ligne par intégration :

| Ligne | Exemple | Ce que ça veut dire |
| --- | --- | --- |
| Réponse Hue | 120 ms (95 % sous 300 ms) | la moitié des ordres sont vus en 120 ms, presque tous en 300 ms |
| Réponse MQTT | 157 ms (95 % sous 3,5 s), 8 ordres, dont 3 au-delà de 3 s | un appareil MQTT traîne : la ligne passe en rouge |

Les 200 derniers ordres de chaque intégration sont gardés. L'API : `GET /api/system`, champ
`devices`.

## Somfy, quatre fois plus vite

La box Somfy ne « pousse » pas ses changements : le serveur va les chercher, toutes les 2 secondes.
Un volet qui bouge pouvait donc s'afficher jusqu'à 2 secondes plus tard. Maintenant, **juste après
un ordre**, la box est relevée **toutes les 0,5 seconde pendant 20 secondes**, puis on revient au
rythme normal. La box n'est donc pas plus chargée le reste du temps.

Mesuré avec la fausse box des tests : un volet fermé s'affichait 2,0 s après l'ordre, il s'affiche
maintenant après 0,51 s. Un test garde ce gain : il échoue au-delà de 1,5 s.

Hue (flux d'événements du pont) et MQTT (messages poussés) n'attendent rien : le changement
arrive dès que l'appareil le signale. Là, le temps qui reste vient de l'appareil et du réseau radio.

## L'appareil qui répond lentement

Quand les **3 derniers ordres** envoyés à un appareil ont tous mis **plus de 3 secondes**, une
alerte de réparation apparaît dans la cloche : « Prise lente répond lentement ». Elle donne le
temps mesuré, et ce qu'il faut vérifier : la pile (voir l'[étape 61](piles.md)), la distance au
relais Zigbee le plus proche, ou au point d'accès Wi-Fi. Elle disparaît dès qu'un ordre aboutit
vite.

Essayé avec un broker MQTT et deux fausses prises, l'une qui répond en 0,15 s, l'autre en 3,5 s :
« Réponse MQTT : 157 ms (95 % sous 3,5 s) », et l'alerte pour la prise lente au passage suivant de
la vérification.

## Sécurité

- **Les chiffres :** seul l'administrateur les voit, comme le reste de la page Système.
- **La mémoire :** rien n'est écrit dans la base. La place est bornée : 200 mesures par intégration,
  256 ordres en attente, 2 000 appareils suivis au plus.
- **Somfy :** le relevé rapide ne dure que 20 secondes après un ordre, et un seul relevé est en cours
  à la fois.

## Comment c'est construit

- `crates/ha-core/src/latency.rs` (nouveau) : `Latencies` reçoit les événements du bus (`observe`) et
  calcule médiane et 95e centile (`summaries`), ainsi que les appareils lents (`slow_devices`).
- `crates/ha-notices` : la tâche des notifications, qui écoute déjà tout le bus, lui passe chaque
  événement. Elle fait les alertes « répond lentement » (`slow_devices`) à sa vérification de
  chaque minute.
- `crates/ha-somfy/src/client.rs` : le relevé rapide après un ordre (`fetch_delay`).
- `crates/ha-api` : le champ `devices` de `/api/system`, et les lignes « Réponse … » de la page
  Système.

### Concepts Rust utilisés

**`tokio::time::Sleep` qu'on peut repousser** : au lieu d'un `interval` au rythme fixe, le relevé
Somfy attend un `sleep` épinglé (`tokio::pin!`). Après chaque relevé, ou dès qu'un ordre part, on le
re-règle avec `fetch.as_mut().reset(…)` sur le prochain instant voulu : 0,5 s ou 2 s.

**`saturating_duration_since`** : si un événement porte un instant un peu plus récent que « maintenant »
(deux horloges lues à quelques microsecondes d'écart), une simple soustraction paniquerait. Celle-ci
rend zéro.

**Un `VecDeque` comme fenêtre glissante** : on ajoute à la fin (`push_back`) et on retire au début
(`pop_front`) quand la fenêtre est pleine. Les 200 dernières mesures restent, sans jamais grandir.
