# Étape 39 : les ambiances météo du dessin, expliquées

![La maison un jour de pluie : le fond « jour · pluie » s'affiche tout seul](images/ambiances-jour-pluie.png)

## Le but

Comme sur les tableaux de domo-blog, la maquette **vit avec le temps qu'il fait**, de jour comme de
nuit, et pour **tous les temps** que donne la météo. Le dessin choisit tout seul le bon fond,
d'après le soleil (`sun.sun`) et la météo (`weather`).

| Temps            | Conditions de la météo                  | S'il manque le fond…  |
| ---------------- | --------------------------------------- | --------------------- |
| Soleil           | `sunny`, `clear-night`                  | (le fond de jour ou de nuit) |
| Éclaircies       | `partlycloudy`                          | soleil                |
| Nuageux          | `cloudy`, `exceptional`                 | soleil                |
| Brouillard       | `fog`                                   | nuageux               |
| Vent             | `windy`, `windy-variant`                | nuageux               |
| Pluie            | `rainy`                                 | nuageux               |
| Averses          | `pouring`                               | pluie                 |
| Orage            | `lightning`, `lightning-rainy`          | pluie                 |
| Grêle            | `hail`                                  | orage                 |
| Neige            | `snowy`                                 | nuageux               |
| Pluie et neige   | `snowy-rainy`                           | neige                 |

Soit 22 fonds possibles par onglet (11 temps, de jour et de nuit), tous facultatifs sauf le fond de
jour.

## Régler les fonds

**Mon compte → Dessin de la maison… → Fonds et ambiances**, pour chaque onglet :

![Les fonds de jour et de nuit pour chaque temps, l'ambiance affichée, et l'ambiance du moment](images/ambiances-reglage.png)

- seul le **fond de jour** est nécessaire ; les autres sont facultatifs ;
- **il en manque un ?** Le plus proche s'affiche : on remonte les temps (grêle → orage → pluie →
  nuageux → soleil) au même moment, puis, la nuit, la même chose de jour. Un fond de nuit seul
  suffit donc à avoir le jour et la nuit, comme avant ; un fond « pluie » sert aussi aux averses,
  aux orages et à la grêle ;
- **Ambiance affichée** : « Automatique », ou une ambiance imposée, pour essayer ses images sans
  attendre la pluie ;
- « Maintenant : nuit, pluie » dit l'ambiance qui serait choisie en ce moment.

Le passage d'une ambiance à l'autre se fait en fondu. Les calques (garage ouvert, lumières…) et
les valeurs restent posés par-dessus.

### Faire les images

Toujours à partir de l'image de jour validée (jamais d'une variante, sinon le dessin dérive), en
demandant à l'IA de garder **exactement** le cadrage :

> Same image, IDENTICAL composition, camera angle and framing. Only change: heavy rain, wet
> reflective ground, overcast sky light.

> Same image, IDENTICAL composition, camera angle and framing. Only change: night time, light snow
> on the roofs and the garden.

![La même maison, de nuit sous la pluie](images/ambiances-nuit-pluie.png)

## Sécurité

- Les fonds d'ambiance sont des images comme les autres : PNG, JPEG ou WebP, vérifiées à leur
  contenu ; une image qui sert à une ambiance ne peut pas être retirée.
- Le serveur n'accepte que les ambiances connues (`jour-` ou `nuit-` puis un des 10 temps), et une
  ambiance imposée parmi les 22.

## Comment c'est construit

- `ha-api/src/drawing.rs` : chaque onglet a `ambiances` (`{"jour-pluie": "image"}`) et `forced`.
- `house.js` : `weatherKind` traduit la condition de la météo en temps ; `ambianceOf` donne
  l'ambiance (ou celle imposée) ; `backgroundFor` remonte la chaîne des temps « parents » jusqu'au
  premier fond qui existe. Le fond d'ambiance est une image posée sur le fond de jour, qui change en fondu.
