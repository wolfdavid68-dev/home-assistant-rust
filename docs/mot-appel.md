# Étape 97 : le mot d'appel « Maison », expliqué

## Le but

Jusqu'ici, pour parler à la tablette, il fallait la toucher : ouvrir Assist, toucher le micro. Les
mains prises (la cuisine, les bras chargés), ce n'est pas pratique.

> Maison, allume la lumière du salon.
> *C'est fait : allumer light.salon.*

ou, en deux temps :

> Maison.
> *(un petit son, le micro s'ouvre : « Oui ? Je t'écoute… »)*
> Ferme les volets.
> *C'est fait.*

Maintenant, si on le coche, la tablette **écoute le mot « Maison »** : elle se réveille quand une
phrase commence par ce mot, comme un Echo avec « Alexa ». Tout reste dans la maison : ce qui est
dit part vers le mini-PC, jamais sur Internet, et rien n'est gardé.

## Comment ça marche

1. Dans **Mon compte**, on coche **Écouter le mot « Maison »** (décoché par défaut, montré seulement
   quand le service d'écoute est installé). Sur cet appareil seulement : la tablette de la cuisine
   peut écouter, celle de l'entrée non.
2. Un **signe reste affiché** en bas à gauche de l'écran, même sur l'écran de veille, tant que c'est
   coché : « À l'écoute de « Maison » » (un micro sur un rond de couleur).
3. La tablette **écoute en continu**, mais n'envoie rien tant que personne ne parle : un petit
   module du navigateur (`ecoute.js`) mesure le niveau du son toutes les 20 ms et apprend le bruit
   de fond de la pièce (le frigo, la hotte). Quand le son dépasse nettement ce bruit, il garde le
   **morceau de parole**, avec 300 ms d'avant pour ne pas couper le début du mot.
4. Le morceau se termine après 0,7 s de silence (6 s au plus). S'il dure au moins 0,4 s, il part
   au mini-PC, en WAV 16 kHz, sur `POST /api/conversation/wake`. Un bruit bref (une porte, une
   cuillère) n'est pas envoyé.
5. Le serveur le fait écouter par Parakeet, comme une phrase dite au micro, et regarde **si la
   phrase commence par « Maison »** :
   - **non** (« il fait beau aujourd'hui », « la maison est rangée ») : la réponse est seulement
     `{"wake": false}`, et le texte est oublié aussitôt ;
   - **« Maison » seul** : `{"wake": true}`. La tablette quitte l'écran de veille, ouvre Assist,
     fait un petit son et ouvre le micro (« Oui ? Je t'écoute… (touche l'écran pour arrêter) ») ;
     si personne ne parle, il se referme au bout de 6 s sans rien envoyer ;
   - **« Maison, allume la cuisine »** : la commande est comprise et obéie tout de suite, comme
     une phrase dite au micro, avec les **mêmes droits**. La tablette ouvre Assist, affiche la
     phrase et la réponse, la dit, puis le micro se rouvre pour une phrase de suite
     ([étape 96](enchainer.md)).
6. Pendant que la tablette écoute une phrase ou parle, l'écoute du mot **se met en pause** (le
   signe le dit : « « Maison » en pause ») ; elle reprend 1,5 s après la fin de la conversation.

Le mot est reconnu avec de la tolérance : majuscules, accents et ponctuation ne comptent pas,
« Ok Maison » et « Dis Maison » marchent, et les petites erreurs d'écoute aussi (« Maisons »,
« Mézon »). Mais pas un autre mot : « raison » ou « saison » ne réveillent pas la tablette, et
« maison » au milieu d'une phrase non plus.

## Le réglage

| Où | Réglage | Rôle |
|----|---------|------|
| Mon compte, sur la tablette | **Écouter le mot « Maison »** | Coché : la tablette écoute le mot (décoché par défaut) |
| Le serveur | `HA_MOT_APPEL=Maison` | Le mot d'appel, 1 à 3 mots, des lettres seulement (« Maison » si rien n'est dit) ; un mauvais réglage est signalé au démarrage et « Maison » reste |

Le mot d'appel demande le **service d'écoute** (`HA_ECOUTE=ws://127.0.0.1:10350`, mis par
`tout-compris.sh --voix`, ou `ws://ecoute:10350` avec le service `ecoute` de Docker, voir l'[étape 84](voix.md)) : il garde
Parakeet en mémoire et répond en une fraction de seconde. Avec le programme lancé pour chaque phrase
(`HA_ECOUTE=/chemin/du/dossier`), le modèle serait relu pour chaque morceau : la case n'est alors
pas montrée, et `/api/conversation/wake` répond 503.

## Sur la tablette murale (Fully Kiosk)

Le navigateur de la tablette doit garder la page ouverte et le micro permis. Dans Fully Kiosk
Browser :

- **Web Content Settings → Enable Microphone Access** : coché (sinon le signe dit « Micro refusé :
  « Maison » n'est pas écouté ») ; Android demande aussi la permission du micro pour Fully, une
  fois ;
- **Web Content Settings → Autoplay Audio** (« Autoplay Videos » selon la version) : coché, pour
  que la tablette puisse écouter et répondre sans être touchée après un redémarrage ;
- **Device Management → Keep Screen On** : coché. Avec l'écran éteint, Android met le navigateur
  en veille et **le mot n'est plus écouté**. L'écran de veille de la maison (l'image floutée et
  l'heure, [étape 89](interface-allegee.md)) garde l'écran allumé et écoute toujours ;
- l'adresse de la maison en **HTTPS** ([https-maison.md](https-maison.md)) : sans HTTPS, le
  navigateur refuse le micro (sauf sur la machine elle-même).

## Les limites

- **Écran éteint, autre application, page cachée** : le navigateur coupe le micro et la tablette
  n'écoute plus ; l'écoute reprend quand la page revient. Le signe passe « en pause ».
- **Après un redémarrage de la tablette** : sans toucher l'écran, un navigateur peut garder le son
  en pause. Le signe le dit : « Touche l'écran pour écouter « Maison » » ; un seul toucher suffit
  ensuite pour toute la visite.
- **Pas un vrai détecteur de mot** : chaque morceau de parole est écouté en entier par Parakeet sur
  le mini-PC. Dans une pièce où l'on parle beaucoup, c'est un morceau toutes les quelques secondes ;
  le mini-PC le supporte (une fraction de seconde chacun), mais pas plus de 30 morceaux par minute
  et par tablette : au-delà, la tablette fait une pause (le temps que le serveur indique) et le
  mot n'est pas entendu pendant ce temps.
- **Une vraie phrase passe toujours avant** : si l'écoute est occupée (quelqu'un parle au micro
  d'une autre tablette), le morceau est abandonné sans attendre. Il suffit de redire « Maison ».
- **La télévision ou la radio** peuvent réveiller la tablette si une phrase y commence par
  « Maison ». Une commande comprise de travers obéit comme si on l'avait dite : on peut changer le
  mot (`HA_MOT_APPEL`) ou décocher la case.
- **Une fenêtre ouverte** (Assist, une carte) passe par-dessus le signe ; il revient dès qu'elle se
  ferme.
- **Un morceau à la fois** : pendant qu'un morceau est écouté, ce qui est dit est abandonné.

## Vie privée et sécurité

- **Rien ne part sur Internet** : les morceaux vont de la tablette au mini-PC, qui les fait écouter
  par Parakeet sur la même machine.
- **Rien n'est gardé** : ni le son, ni le texte, ni sur le disque ni dans les journaux. Sans le mot,
  la réponse est seulement « non ». Le serveur ne retient, en mémoire, que l'heure des morceaux de
  la dernière minute de chaque tablette (pour la limite), sous l'empreinte de son jeton.
- **Coché par appareil, décoché par défaut**, et **un signe toujours visible** tant que c'est
  coché, même sur l'écran de veille. Décocher ferme le micro aussitôt (le voyant du navigateur
  s'éteint).
- **Il faut être connecté** avec un compte de la maison : une commande après « Maison » a les droits
  de ce compte, la [garde](garde.md), le mot de passe redemandé pour une serrure, l'audit, comme une
  phrase au micro. Les jetons d'assistant IA ([étape 55](mcp.md)) sont refusés.
- **Des morceaux courts** : 6,5 s au plus (refusé au-delà, la taille est aussi bornée avant même
  de lire le son), un vrai WAV 16 bits mono 16 000 Hz ; un silence n'est même pas écouté.
- **30 morceaux par minute et par tablette** au plus (réponse 429 avec le temps à attendre).
- **Une vraie phrase passe avant** : un morceau n'attend jamais son tour, pour qu'une tablette
  bavarde ne retarde pas celui qui parle au micro.

## Comment c'est construit

| Fichier | Rôle |
| ------- | ---- |
| [crates/ha-api/src/wake.rs](../crates/ha-api/src/wake.rs) | Nouveau : `POST /api/conversation/wake`, le mot d'appel tolérant (`WakeWord`), la limite par appareil |
| [crates/ha-stt/src/lib.rs](../crates/ha-stt/src/lib.rs) | `transcribe_if_free` (écouter seulement si personne n'attend), `keeps_model` |
| [crates/ha-api/src/voice.rs](../crates/ha-api/src/voice.rs) | `wake` et `wake_word` dans `GET /api/conversation/voice`, pour que la tablette montre la case |
| [crates/ha-server/src/main.rs](../crates/ha-server/src/main.rs) | Le réglage `HA_MOT_APPEL` |
| [web/ecoute.js](../crates/ha-api/web/ecoute.js) | Nouveau module `mot-appel` : le bruit de fond appris, les morceaux de parole |
| [web/voix.js](../crates/ha-api/web/voix.js) | L'écoute du mot, le signe, la pause, le réveil d'Assist, la case |
| [web/index.html](../crates/ha-api/web/index.html), [web/style.css](../crates/ha-api/web/style.css) | La case dans Mon compte et le signe en bas à gauche |

Les tests (`crates/ha-api/tests/api/voice.rs`) utilisent une fausse écoute qui « entend » des phrases
données : « Maison, allume la lumière du salon. » qui donne l'ordre, « Maison. » seul, une phrase
sans le mot, un silence pas même écouté, un morceau trop long ou qui n'est pas un WAV, la limite
de 30 morceaux (et une autre tablette qui passe), un jeton d'assistant refusé, et une vraie phrase
lente qui fait abandonner un morceau en moins d'une demi-seconde.

Essayé dans Chromium, avec une copie du serveur sur un autre port, cette fausse écoute et un micro
simulé (un son qu'on monte et baisse) : la case cochée, le signe « À l'écoute de « Maison » » ; un
bruit de 150 ms pas envoyé ; « Il fait beau » qui répond non ; « Maison. » qui ouvre Assist et le
micro (« Oui ? Je t'écoute… »), puis « Allume la lumière du salon. » obéie, la suite refermée au
bout de 6 s et l'écoute du mot reprise 1,5 s plus tard ; « Maison, éteins la lumière du salon. »
obéie d'un coup, puis « Merci. » en suite ; la page cachée qui met le mot en pause puis le reprend ;
la case décochée qui ferme le micro et cache le signe.

### Concepts utilisés

**Détecter la parole par l'énergie** : le module mesure la force du son (sa moyenne quadratique)
toutes les 20 ms. Il retient le bruit de fond, qui descend vite quand la pièce se calme et remonte
lentement (une voix qui dure ne devient pas « du bruit »). Trois tranches de suite nettement plus
fortes que ce bruit : quelqu'un parle.

**Un mot d'appel tolérant** : la phrase entendue est mise en minuscules, sans accents ni
ponctuation, puis chaque mot est réduit à une prononciation simple (« ai » comme « e », « z »
comme « s », sans le « s » muet) : « Maisons » et « Mézon » deviennent tous deux « meson ». Une
seule lettre de différence est encore acceptée, mais jamais la première.

**Abandonner plutôt qu'attendre** : l'écoute du mini-PC ne fait qu'une chose à la fois. Une phrase
dite au micro attend son tour ; un morceau du mot d'appel, lui, regarde si l'écoute est libre et
abandonne sinon (`try_acquire`). Ainsi celui qui parle n'attend jamais derrière la tablette d'à
côté.
