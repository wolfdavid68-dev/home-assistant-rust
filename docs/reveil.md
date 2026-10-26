# Étape 102 : le réveil en vague, expliqué

## Le but

Toucher la tablette en veille faisait disparaître l'écran de veille d'un coup : un clignement sec,
du noir à la page pleine de tuiles.

Maintenant, l'écran de veille **s'ouvre en cercle depuis le doigt** : un trou aux bords flous
grandit jusqu'aux coins de l'écran, avec une onde de lumière douce sur son bord, et la maison
apparaît dedans. Cela dure 0,7 seconde.

```
   ┌─────────────────────────┐      ┌─────────────────────────┐
   │ veille                  │      │ veille  ╭──────╮        │
   │          ☝              │  →   │       ( tuiles  )       │  →  la page
   │                         │      │         ╰──────╯        │
   └─────────────────────────┘      └─────────────────────────┘
```

## Les règles

- La vague part **du point touché**. Au clavier, elle part du centre.
- Le premier contact ne commande toujours rien : pendant la vague, l'écran de veille avale encore
  les touches, rien ne s'allume sous le doigt.
- Le dessin de la maison retrouve sa place tout de suite ; une copie muette reste dans l'écran de
  veille le temps de la vague, pour qu'il ne se vide pas pendant qu'il s'ouvre.
- Si l'écran de veille revient pendant une vague (rare), elle s'arrête net.
- **Mouvement réduit** (réglage du système) : un simple fondu de 0,3 seconde, sans cercle ni
  onde.

## Ce qui ne change pas

- Pas de photo : la vague est faite de dégradés CSS (un masque qui s'agrandit et un anneau
  lumineux), sans image ni bibliothèque.
- Rien ne part vers le mini-PC.

L'idée vient de l'assistant vocal Cornelius (une transition qui part du geste) ; le code est écrit
pour ce projet, rien n'en est copié.
