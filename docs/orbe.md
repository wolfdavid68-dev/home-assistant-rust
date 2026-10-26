# Étape 98 : l'orbe de la voix, expliqué

## Le but

Quand on parle à la tablette depuis l'autre bout de la pièce, on ne lit pas les petites lignes
« Je t'écoute… » ou « Je réfléchis… ». Il faut voir **de loin**, d'un coup d'œil, ce que fait la
voix.

En haut de la fenêtre Assist, une **boule lumineuse** change de couleur selon ce qui se passe :

| Couleur | Ce que fait la voix |
| --- | --- |
| bleu-gris, qui respire doucement | elle attend |
| turquoise, avec une ligne qui bouge | elle **écoute** : la ligne suit le niveau du micro, le halo grossit quand on parle fort |
| violet, qui tourne | elle **réfléchit** (la phrase est partie au mini-PC) |
| orange, avec des reflets | elle **répond** à voix haute |
| rouge, un instant | ça n'a pas marché (la réponse est une erreur), puis elle revient au repos |

Le petit point du bouton « Parler » (sur l'écran de la tablette murale) prend les mêmes couleurs.

## Comment ça marche

- `voix.js` connaît déjà l'état de la voix (`setVoiceState`) et mesure le niveau du micro pour
  chaque morceau de son (`voiceChunk`). Il les passe à `orbe.js`.
- `orbe.js` met l'état sur la boule (`data-state`) ; les couleurs et les mouvements sont dans la
  feuille de style. La ligne est un petit dessin SVG redessiné au plus une fois par image, avec les
  28 derniers niveaux du micro : les bords restent calmes pour qu'elle entre et sorte de la boule.
- Une réponse en erreur (`assistSay`, depuis la maison) fait passer la boule au rouge 2,5 s, sauf
  si la voix est déjà en train d'écouter.
- Toucher la boule fait comme le bouton micro : c'est une grande cible pour le doigt. Pour les
  lecteurs d'écran, la boule est cachée (`aria-hidden`) et le bouton micro reste la commande
  annoncée.
- La boule n'apparaît que si la voix est installée sur le mini-PC.

## Ce qui ne change pas

- Rien de nouveau ne part vers le mini-PC : la boule montre seulement ce que la page sait déjà.
- Pas d'image ni de bibliothèque : des dégradés CSS et un tracé SVG construit sans `innerHTML`.
- **Mouvement réduit** (réglage du système) : la boule ne bouge plus et ne tourne plus, seules les
  couleurs changent.

L'idée vient de l'assistant vocal Cornelius (un orbe qui change de couleur) ; le code est écrit
pour ce projet, rien n'en est copié.
