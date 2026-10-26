# Étape 104 : une fenêtre qui s'allume avec sa pièce, expliqué

## Le but

Sur la maquette de nuit, chaque fenêtre éclairée est un calque. Jusqu'ici, un calque suivait
**un seul appareil** : `light.plafonnier`. Mais une pièce a souvent plusieurs lampes, et la fenêtre
doit s'éclairer dès que **l'une d'elles** est allumée.

Poser la même image une fois par lampe ne marche pas : quand trois lampes sont allumées, l'image
est dessinée trois fois, et son halo devient trop fort (on voit même le rectangle de la découpe).

Un calque peut donc maintenant suivre **une pièce** de `areas.yaml` : il est visible dès qu'une
lumière de la pièce est allumée, et il n'est dessiné qu'une fois.

## Le régler

**Mon compte → Dessin de la maison… → Calques**, pour chaque fenêtre :

1. « + Ajouter un calque », puis choisis l'image de la fenêtre (une image transparente, avec
   seulement la fenêtre éclairée) ;
2. « Suit » : **Une pièce**, puis la pièce (Cuisine, Bureau…) ;
3. « Moment » : **La nuit** (ou Toujours).

Dans la configuration, un calque de pièce s'écrit :

```json
{"image": "fenetre_cuisine", "area": "Cuisine", "when": "night"}
```

## Sécurité

- Le serveur vérifie qu'un calque suit **un appareil ou une pièce** (l'un des deux), que la pièce
  existe dans `areas.yaml`, et qu'un calque de pièce n'a ni états, ni « sauf », ni position.
- Un **invité** garde les calques de pièce, comme les zones de pièce : le navigateur ne compte que
  les lampes que le compte peut voir. Une pièce dont toutes les lampes lui sont cachées reste
  donc sombre.

## Comment c'est construit

- `crates/ha-api/src/drawing.rs` : `Layer` a un champ `area`, et `entity_id` devient facultatif ;
  `check_page` et `check_drawing` vérifient le reste.
- `crates/ha-api/web/house.js` : `roomLit` regarde si une lumière de la pièce est allumée, et
  `layerShown` s'en sert pour un calque de pièce. L'éditeur propose « Suit : un appareil / une
  pièce ».

```bash
cargo test -p ha-api
```
