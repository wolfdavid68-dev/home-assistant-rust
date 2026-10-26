# Étape 94 : les surnoms des appareils, expliqué

## Le but

Dans la maison, personne ne dit « allume Lampe salon ». On dit « allume la lampe du canapé »,
« la grande lumière », « ferme le store ». Home Assistant permet de donner à chaque appareil des
**autres noms** (les « alias ») qu'Assist comprend comme le vrai nom.

Ici, c'est pareil : un appareil garde son nom affiché, et reçoit en plus une liste de surnoms.

## Mettre en place

Les autres noms se donnent là où l'on donne déjà le nom et la pièce : dans `areas.yaml`
([étape 9](pieces.md) ; avec Docker : `config/areas.yaml`), avec la clé `aliases` :

```yaml
- area: Salon
  entities:
    - entity: light.salon
      name: Lampe
      aliases: [la lampe du canapé, la grande lumière]
    - entity: cover.volet_salon
      name: Volet
      aliases: [le store]

- area: Chambre
  entities:
    - {entity: cover.volet_chambre, aliases: [le store]}
```

Puis vérifie et redémarre :

```powershell
cargo run -p ha-server -- areas check
```

La vérification refuse un autre nom vide, trop long (60 caractères), avec un caractère de
contrôle, en double pour le même appareil (majuscules comprises), ou plus de dix par appareil. Une
faute de frappe dans la clé (`alias:` au lieu de `aliases:`) est refusée elle aussi. Un même
surnom donné à deux appareils est permis, mais signalé :

```
  attention : « le store » est l'autre nom de cover.volet_salon, cover.volet_chambre ; Assist demandera lequel
```

Il n'y a pas d'écran pour les modifier depuis le tableau de bord : les noms et les pièces ne se
règlent aujourd'hui que dans `areas.yaml`, les surnoms suivent le même chemin.

## Ce qu'Assist en fait

Un autre nom se reconnaît **exactement comme le nom** ([Assist](assist.md)) : sans tenir compte
des accents, des majuscules ni des petits mots (« le », « la », « du »…), au singulier comme au
pluriel. Avec l'exemple plus haut :

| Phrase | Appareil |
| --- | --- |
| allume la lampe du canapé | Lampe (salon) |
| éteins la Grande Lumiere | Lampe (salon) |
| ferme le store du salon | Volet (salon) : la pièce dite départage |
| ferme le store de la chambre | le volet de la chambre |
| ferme le store | « Lequel ? Volet, … » : deux appareils portent ce surnom |
| ferme les stores | les deux volets (au pluriel, tous ceux qui portent ce nom) |

Comme pour le nom, le nom le plus long l'emporte : « la lampe du canapé » passe avant une autre
lampe nommée seulement « Lampe ».

**La pièce dite compte aussi** : si l'on dit « ferme le store de la chambre » et qu'un seul store
s'appelle ainsi mais se trouve au salon, ce n'est pas lui qui est fermé, sauf si son nom contient
lui-même le nom de la pièce. Cette règle vaut pour les noms comme pour les surnoms.

## Jamais plus de droits

Un surnom n'est qu'une autre façon de désigner un appareil que le compte voit déjà :

- Assist ne cherche que parmi les appareils **visibles** par le compte. La serrure d'un invité
  reste introuvable, qu'on l'appelle « la serrure » ou « la grosse porte » ;
- l'ordre passe ensuite par le **même chemin qu'un toucher** : un invité qui dit « allume la lampe
  du canapé » dans une pièce protégée voit son ordre attendre une validation
  ([garde](garde.md)), et déverrouiller redemande toujours le mot de passe ;
- deux appareils du même surnom donnent « Lequel ? », jamais les deux à la fois au singulier.

## Ailleurs que dans Assist

- **La voix** sur la tablette ([étape 84](voix.md)) passe par Assist : les surnoms y marchent tels
  quels.
- **L'assistant MCP** ([mcp.md](mcp.md)) : `get_house_state` montre les surnoms à côté du nom
  (`- Lampe (light.salon) [autres noms : la lampe du canapé, la grande lumière] : …`), et
  l'outil `assist` les comprend.
- **Le WebSocket**, comme le registre des appareils de Home Assistant :
  - `{"id": 1, "type": "config/entity_registry/get", "entity_id": "light.salon"}` ;
  - `{"id": 2, "type": "config/entity_registry/get_entries", "entity_ids": ["light.salon", …]}`
    (500 au plus) ;

  donnent pour chaque appareil `entity_id`, `name` (celui d'`areas.yaml`), `original_name` (celui
  de l'intégration), `aliases`, `area_id` (la pièce, en identifiant : `salon`) et `platform`
  (l'intégration). Un appareil inconnu ou invisible pour le compte donne `not_found` (ou `null`
  dans `get_entries`) : un invité n'apprend rien de plus.
- **Alexa** ([étape 14](alexa.md)) : l'Echo voit chaque appareil comme une ampoule Hue sous un
  seul nom (`alexa.yaml`), et c'est l'appli Alexa qui gère ses propres surnoms (groupes, routines).
  Les surnoms d'`areas.yaml` n'y sont donc pas envoyés.

## Côté code

- `crates/ha-areas` : `AreaEntity.aliases`, vérifiés à la lecture (`check_aliases`) ;
  `shared_aliases` liste les surnoms portés par plusieurs appareils (pour `areas check`).
- `crates/ha-assist` : `Device.aliases` ; la recherche par nom essaie le nom puis chaque surnom,
  et écarte un appareil d'une autre pièce que celle dite.
- `crates/ha-api` : `assist::devices` ajoute les surnoms des appareils visibles ; `ws.rs` sert
  `config/entity_registry/get` et `get_entries` ; `mcp.rs` les montre dans `get_house_state`.
