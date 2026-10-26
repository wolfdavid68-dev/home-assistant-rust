# Étape 101 : l'âge des données, expliqué

## Le but

« Salon : 19,5 °C », mais depuis quand ? Si le capteur n'envoie plus rien depuis la nuit, le chiffre
affiché est vieux, et rien ne le disait. Et quand la tablette perdait la maison (Wi-Fi coupé,
mini-PC redémarré), on ne le voyait qu'au petit mot « hors ligne » dans un coin.

Maintenant, chaque valeur dit son heure, et une coupure se voit d'un coup d'œil, sans fenêtre qui
s'ouvre.

## Sous les valeurs

Sous chaque tuile de capteur (et sur les cartes en verre de l'étape 99) :

| Situation | Ce qui est écrit |
| --- | --- |
| la valeur est récente | « relevé à 08:15 » (ou « relevé le 24/10 à 22:40 » si c'est un autre jour) |
| la valeur est trop vieille | « dernière donnée il y a 12 minutes », en **ambre**, avec un petit point |

Une valeur est « trop vieille » au bout de :

- **15 minutes** pour une puissance, un courant ou une tension (W, kW, VA, A, V) : ces capteurs
  envoient souvent ;
- **4 heures** pour une température ou une humidité : une température qui ne bouge pas n'est pas
  renvoyée, il faut laisser de la marge ;
- jamais pour le reste (un compteur, une date, un état) : il peut rester le même des jours.

Le texte se met à jour seul chaque minute.

## La ligne d'état

Quand la tablette perd le lien avec la maison **plus de 3 secondes**, une pastille ambre apparaît
en haut de l'écran :

```
        ● Connexion perdue avec la maison, nouvel essai… · dernière donnée à 08:15
```

- Elle reste au-dessus de tout, écran de veille compris, mais ne prend pas le doigt : on peut
  toujours toucher l'écran dessous.
- La page continue de réessayer seule, comme avant ; la pastille part d'elle-même au retour du
  lien.
- Une coupure de moins de 3 secondes (un mini-PC qui redémarre vite) ne montre rien.
- Les lecteurs d'écran l'annoncent une fois (`role="status"`).

## Ce qui ne change pas

- Rien de nouveau ne part vers le mini-PC : l'heure vient de `last_updated`, que la maison envoie
  déjà avec chaque valeur.
- Le petit mot « en ligne » / « hors ligne » de l'en-tête reste.
- **Mouvement réduit** : la pastille apparaît sans glisser et son point ne clignote pas.

L'idée vient de l'assistant vocal Cornelius (montrer l'âge d'une donnée et l'état du lien) ; le
code est écrit pour ce projet, rien n'en est copié.
