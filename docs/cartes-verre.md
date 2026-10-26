# Étape 99 : les cartes en verre, expliqué

## Le but

Une réponse comme « Salon : 19,5 °C ; Cuisine (Rez) : 20 °C » se lit mal de loin dans le fil de la
conversation. Le chiffre est ce qui compte : il doit se voir **en grand**.

Maintenant, quand la maison répond avec un chiffre, une heure ou une liste, une **carte en verre**
sort de l'orbe, sous elle, dans la fenêtre Assist :

```
┌──────────────────────────────┐
│ • Salon                      │   ← d'où vient la valeur
│ 19,5 °C                      │   ← en grand
│ relevé à 08:15               │
└──────────────────────────────┘
┌─────────────┐ ┌──────────────┐
│ • Cuisine   │ │ • Lave-linge │
│ 20 °C       │ │ 11:20        │
│             │ │ en marche,   │
│             │ │ fini vers    │
└─────────────┘ └──────────────┘
```

## Les règles

- **Trois cartes au plus** : la plus récente en grand, les deux d'avant plus petites, dessous. Une
  nouvelle réponse pousse les plus vieilles.
- Chaque carte **s'efface après une minute**, en fondu.
- **Fermer Assist** (fin de la conversation) fait partir toutes les cartes.
- Le petit titre dit **d'où vient** la valeur : le nom donné par la maison (« Cuisine (Rez) »,
  « Lave-linge »), ou « Assist » pour une phrase entière.
- Quand ce nom est celui d'un appareil connu, la carte dit aussi l'heure de la dernière donnée :
  « relevé à 08:15 ».

## Ce qui fait une carte

La réponse d'Assist n'est que du texte : `cartes.js` le relit, sans rien demander de plus.

| Réponse | Carte |
| --- | --- |
| « Salon : 19,5 °C ; Cuisine (Rez) : 20 °C. » | une carte par pièce (trois au plus) |
| « Lave-linge : en marche, fini vers 11:20. » | **11:20**, « en marche, fini vers » dessous |
| « Thermostat : chauffe, consigne 21 °C, il fait 19,5 °C. » | **19,5 °C** (la mesure passe avant la consigne) |
| « À la maison : Alice, Bob et Chloé. » | trois pastilles : Alice, Bob, Chloé |
| « Conso : 450 W ; Aujourd'hui : 3,2 kWh. » | deux cartes |
| « Il fait 19,5 °C dans le salon. » | **19,5 °C**, la phrase entière dessous |
| « C'est fait : allumer light.salon. » | rien : c'est une confirmation |
| une erreur | rien : l'orbe rougit (étape 98) |

Les unités reconnues : °C, %, W, kW, kWh, VA, €, km/h, mm, lx, ppm, min, h.

## Ce qui ne change pas

- Le fil de la conversation garde toute la réponse ; les cartes la répètent seulement en grand.
  Elles sont cachées pour les lecteurs d'écran (`aria-hidden`), qui lisent déjà le fil.
- Rien n'est gardé : les cartes vivent dans la page et partent avec elle.
- Pas d'`innerHTML` : chaque texte passe par `textContent`.
- **Mouvement réduit** : les cartes apparaissent et partent en fondu simple, sans glisser.

L'idée vient de l'assistant vocal Cornelius (des cartes qui accompagnent la réponse) ; le code est
écrit pour ce projet, rien n'en est copié.
