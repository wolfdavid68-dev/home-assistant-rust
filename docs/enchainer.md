# Étape 96 : enchaîner les phrases à la voix, expliqué

## Le but

Jusqu'ici, chaque phrase dite à la tablette était seule : il fallait retoucher le micro, et
redire tout le nom de l'appareil.

> Allume la lumière du salon.
> *C'est fait.*
> Et la cuisine aussi.
> *C'est fait.*
> Éteins-la.
> *C'est fait.*
> Merci.
> *Avec plaisir.*

Maintenant, après la réponse dite, **le micro se rouvre tout seul quelques secondes** pour une
phrase de suite, et Assist **se souvient de la phrase d'avant** : « et la cuisine aussi », « pareil
dans la chambre », « éteins-la », « mets-la à 30 % ». Comme dans Home Assistant (le
`conversation_id` d'Assist et sa « conversation continue »).

## Comment ça marche

1. On touche le micro et on dit sa phrase, comme à l'[étape 84](voix.md).
2. La réponse revient avec un **numéro de conversation** (`conversation_id`), et
   `continue_conversation: true` si la maison peut écouter la suite.
3. Quand la tablette **a fini de parler**, un petit son doux (sur la tablette seulement) et le
   micro qui **bat doucement en rouge** disent qu'elle écoute encore : « Je t'écoute encore…
   (touche l'écran pour arrêter) ».
4. On enchaîne dans les **6 secondes**, ou on ne dit rien : le micro se referme alors **sans rien
   envoyer**, ni message d'erreur.
5. La phrase de suite part avec le même numéro ; le serveur retrouve le contexte, Assist la
   complète et donne l'ordre.

Ce qui arrête tout, tout de suite :
- **« merci »**, **« c'est tout »**, **« stop »**, **« non merci »**, « ça ira », « au revoir »… :
  une réponse courte (« Avec plaisir. », « D'accord. »), et la conversation est oubliée ;
- **toucher l'écran** ou le bouton pendant que la tablette écoute la suite ou parle ;
- fermer Assist, ou la page cachée (autre application, écran éteint) ;
- **5 phrases de suite** au plus après la première (le serveur, lui, en compte 6 par
  conversation et dit ensuite de ne plus écouter).

Le micro ne se rouvre **jamais pendant que la tablette parle** : seulement quand la réponse est
finie. Taper une phrase dans Assist continue aussi la conversation (« éteins-la » après « allume la
lumière du salon »).

## Ce qu'Assist comprend en suite

| Avant | Puis | Il fait |
|-------|------|---------|
| Allume la lumière du salon | Et la cuisine aussi | Allume la lumière de la cuisine (même ordre, même type d'appareil, autre lieu) |
| Allume la lumière du salon | Pareil dans la chambre | Pareil |
| Allume la lumière du salon | Éteins-la | Éteint la même lumière (« le », « la », « les », « tout ») |
| Allume la lumière du salon | Mets-la à 30 % (ou « et à 30 % ») | Règle la même lumière |
| Allume la lumière du salon | Et 50 % dans la chambre | La lumière de la chambre à 50 % |
| Ferme le volet du salon | Et la chambre | Le volet de la chambre |

Une **phrase complète** est toujours comprise pour elle-même : « allume la cuisine » après une
question sur la température n'a pas besoin du contexte. Le contexte ne sert que si la phrase seule
ne suffit pas.

Les **garde-fous** :
- un **lieu inconnu** n'est jamais rabattu sur l'appareil d'avant : « éteins le garage » sans
  garage répond qu'il n'a pas trouvé, il n'éteint pas le salon ;
- un appareil qui **n'est plus visible** pour le compte (droits changés entre deux phrases) n'est
  pas visé ;
- les **questions** (« et dans la chambre ? » après « quelle température… ») ne sont pas
  enchaînées : il faut redire la question entière ;
- les listes (« ajoute du pain ») ne servent pas de contexte.

## Le réglage

Dans **Mon compte**, à côté des réglages de la tablette : **Enchaîner les phrases à la voix**
(coché par défaut, montré quand l'écoute est installée). Sur cet appareil seulement : on peut le
couper sur la tablette de l'entrée et le garder sur celle de la cuisine.

## Sécurité

- **Les mêmes droits qu'une phrase entière** : une suite est reconstruite en ordres complets
  (« éteindre light.plafonnier_cuisine »), puis passe par exactement le même chemin qu'un toucher :
  droits du compte, [garde](garde.md), mot de passe redemandé pour une serrure, audit. Un invité qui
  dit « et le garage » après un volet autorisé est refusé comme s'il l'avait demandé en entier
  (« Pas fait : … »).
- **Une conversation appartient à son compte** : un autre compte qui présente le numéro en commence
  une nouvelle, sans rien voir de l'autre.
- **Le numéro est tiré au hasard** par le système (128 bits, 32 chiffres hexadécimaux) ; un numéro
  mal formé est ignoré, un paramètre inconnu refusé.
- **Rien n'est écrit** : le serveur ne garde, en mémoire seulement, que le petit contexte d'Assist
  (le verbe, la valeur, les appareils visés), jamais le texte dit ni le son, **une minute** après la
  dernière phrase, 64 conversations au plus (les plus vieilles partent). Rien sur le disque, rien
  dans les journaux ; un redémarrage oublie tout.
- **Le micro qui se rouvre se voit et s'arrête d'un toucher** ; il ne se rouvre que si Assist est
  ouvert et la page visible, et jamais au-delà de 5 phrases de suite.

## Comment c'est construit

| Fichier | Rôle |
| ------- | ---- |
| [crates/ha-assist/src/suite.rs](../crates/ha-assist/src/suite.rs) | Nouveau : le contexte (`Context`), `understand_with` (une phrase avec la précédente), les mots qui terminent (`is_goodbye`) ; les fonctions de correspondance des noms ne changent pas |
| [crates/ha-api/src/conversation.rs](../crates/ha-api/src/conversation.rs) | Nouveau : les conversations en mémoire (numéro aléatoire, une minute, 64 au plus, 6 phrases, propriétaire) |
| [crates/ha-api/src/assist.rs](../crates/ha-api/src/assist.rs) | `conversation_id` reçu et rendu, `continue_conversation`, « merci » |
| [crates/ha-api/src/voice.rs](../crates/ha-api/src/voice.rs) | `POST /api/conversation/audio?conversation_id=…` |
| [web/voix.js](../crates/ha-api/web/voix.js) | Le micro rouvert après la réponse (6 s), le petit son, l'arrêt d'un toucher, le réglage |
| [web/account.js](../crates/ha-api/web/account.js) | Le numéro de conversation gardé 50 s par Assist, pour les phrases tapées aussi |

Essayé dans Chromium, avec une copie du serveur sur un autre port, une fausse écoute qui « entend »
des phrases données et un micro simulé : « Allume la lumière du salon. », le micro rouvert tout seul
(« Je t'écoute encore… »), « Éteins-la. » qui éteint la même lumière, puis sans rien dire le micro
refermé au bout de 6 s sans rien envoyer ; « Merci. » qui répond « Avec plaisir. » sans rouvrir ;
un toucher de l'écran qui referme aussitôt ; le réglage coupé, plus d'enchaînement.

### Concepts utilisés

**Le contexte d'une conversation** : Assist ne garde pas les mots, mais ce qu'il en a compris (le
verbe « allumer », peut-être une valeur, le type d'appareil et les appareils visés). « Et la cuisine
aussi » n'a ni verbe ni type : Assist reprend ceux d'avant et cherche dans la cuisine. « Éteins-la »
n'a qu'un verbe et un pronom : Assist reprend les appareils d'avant.

**Un numéro de conversation** : comme un ticket au guichet. La tablette le renvoie avec la phrase
suivante, le serveur retrouve le contexte. Tiré au hasard (comme un jeton) et lié au compte, il ne
sert à rien à quelqu'un d'autre.
