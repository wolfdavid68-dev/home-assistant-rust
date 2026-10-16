# Étape 62 : le délestage, expliqué

## Le but

Le four, la plaque et le chauffe-eau marchent en même temps, et le disjoncteur du compteur saute.
Il faut alors aller le réarmer, et tout redémarre en même temps. Le **délestage** l'évite : quand
la maison tire plus que l'abonnement ne le permet, le serveur coupe d'abord les appareils les moins
utiles, un par un. Il les rallume tout seul quand la consommation est redescendue. Les délesteurs
vendus pour les tableaux électriques font la même chose, mais ici les appareils et leur ordre se
règlent dans un fichier.

## Mettre en place

Copie [delestage.example.yaml](../delestage.example.yaml) en `delestage.yaml` (ou indique son
chemin dans `HA_DELESTAGE_PATH`) :

```yaml
mesure: sensor.linky_intensite   # par défaut
# limite: 30                     # sans elle : la puissance souscrite lue sur le Linky
appareils:                       # coupés dans cet ordre, rallumés dans l'ordre inverse
  - switch.chauffe_eau
  - climate.bureau
  - switch.radiateur_salle_de_bain
retour_apres: 5                  # minutes
```

Vérifie le fichier avec `ha-server delestage check`, redémarre le serveur, puis **arme** le délestage
avec `switch.delestage`, depuis le tableau de bord ou une automatisation.

| Réglage | Ce que c'est |
| --- | --- |
| `mesure` | Ce qui est surveillé : l'intensité du Linky (étape 32, en A) par défaut, ou une puissance (`sensor.linky_puissance`, en VA ; une pince en W marche aussi). |
| `limite` | Le seuil, dans l'unité de la mesure. Sans lui, c'est la **puissance souscrite** lue sur le compteur : 30 A en mode historique, ou 6 kVA (6000 VA) en mode standard. |
| `appareils` | De 1 à 20, du moins utile au plus utile : prises, lumières, ventilateurs, interrupteurs virtuels et thermostats. |
| `retour_apres` | Le temps passé sous la limite avant de rallumer, de 1 à 60 minutes. |

**Ne mets jamais** dans la liste un congélateur, un réfrigérateur, une pompe de relevage ou une box
Internet : ce qu'on coupe doit pouvoir attendre.

## Comment il décide

- **La maison dépasse la limite :** le premier appareil de la liste qui est allumé est coupé.
  Si elle la dépasse encore **15 secondes** plus tard, c'est le suivant, et ainsi de suite. Les
  15 secondes laissent au Linky le temps de mesurer l'effet de la coupure (il publie
  l'intensité toutes les 10 secondes).
- **Un thermostat** est mis à l'arrêt (`hvac_mode: off`). Il retrouve ensuite son mode d'avant
  (`heat`…).
- **La consommation redescend :** quand elle est restée **sous 85 % de la limite** pendant
  `retour_apres` minutes, le dernier appareil coupé est rallumé. Les autres suivent, une minute
  d'écart à chaque fois. À 99 % de la limite, rallumer ferait recouper aussitôt : d'où les 85 %.
- **Un appareil rallumé à la main** pendant ce temps n'est plus touché : il n'est ni recoupé, ni
  rallumé une seconde fois.
- **Si le compteur se tait** (module débranché), rien n'est coupé. Ce qui l'était est rallumé après
  `retour_apres` minutes : mieux vaut un disjoncteur qui saute qu'un chauffe-eau éteint pour
  toujours.
- **Désarmer** `switch.delestage` rallume tout de suite ce qui avait été coupé.
- **Après un redémarrage du serveur**, il reste armé et retrouve ses coupures en cours.

L'interrupteur montre ce qui est coupé, dans son attribut `coupes` (avec ce que faisait chaque
appareil avant). Le journal de bord dit pourquoi un appareil s'est éteint ou rallumé : « par l'automatisation
« Délestage » ».

## Être prévenu

Chaque coupure et chaque remise en marche publie l'événement `delestage`, avec `action` (`coupe` ou
`rallume`), `entity_id`, `mesure` et `limite` :

```yaml
- id: delestage_appli
  alias: Prévenir d'un délestage
  triggers:
    - trigger: event
      event_type: delestage
      event_data:
        action: coupe
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: Délestage
        message: >-
          Un appareil vient d'être coupé pour ne pas faire sauter le disjoncteur
          ({{ states('sensor.linky_intensite') }} A).
        tag: delestage
```

Le détail (quel appareil, quelle mesure) est dans le journal de bord, et dans l'attribut `coupes`
de `switch.delestage`.

## Sécurité

- **Les droits :** un invité ne voit pas `switch.delestage` et ne peut pas le commander. La
  famille et l'administrateur, si.
- **Les invités et la simulation de présence :** la même règle couvre maintenant
  `switch.simulation_presence` (étape 58). Avant, un invité pouvait la voir, alors qu'elle dit quand
  la maison est vide.
- **Les ordres :** ils passent par les vérifications du noyau, comme ceux d'une automatisation. Seuls
  des prises, lumières, ventilateurs, interrupteurs virtuels et thermostats sont acceptés : jamais
  une serrure, un volet ou une alarme.
- **La mesure :** elle vient d'un appareil, donc elle est vérifiée (un nombre positif). Une mesure
  illisible ne coupe rien.

## Comment c'est construit

- `crates/ha-shedding` (nouveau) :
  - `parse_config` lit `delestage.yaml` ;
  - `subscribed_limit` tire la limite du Linky ;
  - `Shedder` décide quoi couper ou rallumer ;
  - `Shedding` est la tâche de fond (vérification toutes les 2 secondes, interrupteur, événement).
- `crates/ha-api/src/roles.rs` : `switch.delestage` et `switch.simulation_presence` cachés aux
  invités.
- `crates/ha-server` : `HA_DELESTAGE_PATH`, `ha-server delestage check`.

### Concepts Rust utilisés

**Une décision sans horloge** : `Shedder::step` reçoit l'instant présent en paramètre (`now`) au lieu
de le lire lui-même. Les tests font défiler le temps (« 15 secondes plus tard », « 5 minutes
plus tard ») sans attendre, et vérifient chaque décision.

**`Option::get_or_insert`** : `calm_since` note le début du calme la première fois qu'on le voit, et
le garde ensuite. Dès que la mesure remonte, il revient à `None`.

**`serde` pour reprendre après un redémarrage** : les coupures (`Vec<Cut>`) sont rangées dans un
attribut de l'interrupteur, lui-même gardé dans la base. Au démarrage, `serde_json::from_value` les
relit. Une valeur abîmée donne simplement une liste vide.
