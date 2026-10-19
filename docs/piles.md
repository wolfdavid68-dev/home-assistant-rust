# Étape 61 : les piles faibles, expliqué

## Le but

Un détecteur de fumée, une sonde de fuite ou un capteur de porte qui n'a plus de pile ne prévient
plus de rien, et souvent sans bruit. Le serveur surveille donc **toutes les piles de la maison** et
prévient avant qu'elles ne lâchent, comme l'extension « Battery Notes » de Home Assistant. Il n'y a
rien à régler.

## Ce qui est surveillé

| Ce que l'appareil envoie | Exemple | Pile faible quand… |
| --- | --- | --- |
| un capteur de classe `battery`, en % (Zigbee2MQTT, Shelly…) | `sensor.fumee_salon_batterie` = `17` | ≤ 20 % |
| un détecteur de classe `battery` (« pile faible ») | `binary_sensor.sonnette_pile` = `on` | `on` |
| l'attribut `battery_level` d'un autre appareil | une serrure, `battery_level: 12` | ≤ 20 % |

Ce qui n'est pas compté :
- un **aspirateur** se recharge tout seul : sa batterie ne compte pas ;
- un appareil **indisponible** a déjà son alerte « ne répond plus » (étape 31) ;
- une valeur absurde (hors de 0 à 100) est ignorée.

## Les alertes

Chaque minute, le serveur vérifie les piles. Une pile faible devient une **alerte de réparation**
dans la cloche, comme un appareil injoignable :

| Niveau | Alerte | Sur le téléphone (étape 50) |
| --- | --- | --- |
| 20 % ou moins, ou « pile faible » | « Pile faible : Détecteur de fumée du salon » (17 %) | non, dans la cloche seulement |
| 5 % ou moins | « Pile presque vide : Détecteur de fuite de la cave » (4 %) | **oui**, catégorie « alertes du serveur » |

- **La pile qui baisse :** une alerte « pile faible » qui passe à 5 % devient grave. Le téléphone
  sonne alors, une seule fois.
- **La pile changée :** l'alerte reste jusqu'à ce que la pile dépasse **30 %**. Une mesure qui hésite
  entre 20 et 21 % ne fait donc pas apparaître et disparaître l'alerte toutes les minutes.
  Une pile neuve la fait disparaître, et une alerte grave devient « Résolu » sur le téléphone.
- **L'alerte ignorée :** fermée dans la cloche, elle ne revient pas tant que la pile n'est pas
  changée.

## Toutes les piles d'un coup d'œil

**Mon compte → Piles** liste toutes les piles de la maison, **la plus faible d'abord** :

- point rouge : 5 % ou moins, à changer tout de suite ;
- point jaune : 20 % ou moins, ou « pile faible », à changer bientôt ;
- point vert : pile bonne.

La liste est réservée aux comptes qui voient tous les appareils : pas aux invités.

## Dans une automatisation

Les alertes suffisent le plus souvent. Pour autre chose (par exemple prévenir un seul appareil),
les capteurs de pile servent comme les autres :

```yaml
- id: pile_detecteur_fumee
  alias: Pile du détecteur de fumée
  triggers:
    - trigger: numeric_state
      entity_id: sensor.fumee_salon_batterie
      below: 15
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: Détecteur de fumée
        message: "Pile à {{ states('sensor.fumee_salon_batterie') }} % : change-la."
        tag: pile_fumee
```

## Sécurité

- Rien n'est envoyé hors de la maison, et rien n'est commandé : le serveur ne fait que lire.
- Les niveaux viennent des appareils : ils sont vérifiés (un nombre entre 0 et 100) avant usage.
- Les alertes sont limitées comme les autres (100 au plus), et leur texte est nettoyé.

## Comment c'est construit

- `crates/ha-notices` :
  - `battery` lit la pile d'une entité ;
  - `low_batteries` fait la liste des alertes ;
  - la vérification de chaque minute les met à jour avec `reconcile`, comme pour les appareils
    injoignables.
- `crates/ha-push` : une alerte affichée qui devient grave (`worsened`) sonne comme une nouvelle.
- `crates/ha-api/web/app.js` : la liste « Piles » de Mon compte (`batteryOf`, mêmes règles que le
  serveur).

### Concepts Rust utilisés

**Un `enum` qui porte des données** (`Battery::Level(f64)` ou `Battery::Low(bool)`) : un capteur
donne un pourcentage, un détecteur seulement « faible ou pas ». Le `match` qui fabrique l'alerte doit
traiter les deux cas, et le compilateur refuse d'en oublier un.

**Des gardes dans un `match`** (`Battery::Level(level) if level <= BATTERY_EMPTY`) : les branches sont
essayées dans l'ordre. « Presque vide » passe donc avant « faible », et `_ => return None` couvre
tout le reste.

**Une hystérésis sans mémoire en plus** : pour savoir si une pile à 22 % garde son alerte, le serveur
regarde si l'alerte existe déjà dans la machine à états. L'état de la maison sert de mémoire, et
rien d'autre n'est à garder ni à sauvegarder.
