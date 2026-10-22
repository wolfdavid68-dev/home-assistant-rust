# Étape 88 : la réponse vocale et la tablette plus rapides, expliquées

## Le but

Deux lenteurs se sentaient au quotidien :
- **la voix** : entre la fin de la phrase et le début de la réponse, il fallait 5 à 6 secondes ;
- **la tablette murale** : chaque mesure d'un capteur (le compteur Linky en envoie toutes les
  1 à 2 secondes) faisait redessiner presque toute la page, de quoi faire saccader une tablette
  modeste.

## Les mesures

### La voix

Du silence qui termine la phrase au début de la réponse, sur le PC d'essai (4 cœurs, comme le
Ryzen 5300U du mini-PC) :

| | Avant (étape 84) | Maintenant |
|---|---|---|
| La tablette attend la fin de la phrase | 1,2 s de silence | **0,9 s** |
| L'écoute (le son devient du texte) | 2,8 à 4,3 s (Whisper « small ») | **0,4 à 0,7 s** (Parakeet) |
| Assist comprend et donne l'ordre | quelques millisecondes | quelques millisecondes |
| La voix prépare la réponse | 0,9 à 1,4 s (Piper relancé à chaque phrase) | **0,1 à 0,3 s** (Piper gardé en marche) |
| **En tout** | **≈ 5,5 s** | **≈ 1,8 s** |

Mesuré de bout en bout par les adresses de la tablette : « Ajoute du pain à la liste de courses »
entendu et compris en 0,53 s, la réponse prête en 0,26 s.

### La tablette

Sur la [vue hub](hub.md), avec un processeur ralenti 4 fois (comme une tablette d'entrée de
gamme) et deux mesures par seconde :

| | Avant | Maintenant |
|---|---|---|
| Travail par mesure | 94 ms | **22 ms** |
| dont la mise en page | 30 ms | **0 ms** |
| Processeur occupé | 27,5 % | **6 %** |
| Fichiers redemandés à chaque ouverture | 20 (réponses « pas changé ») | **0** (seule la page est revérifiée) |

Sur le tableau de bord complet : 44 ms par mesure au lieu de 15 % du processeur, maintenant 27 ms
et 10 %.

## Ce qui a changé

### L'écoute : Parakeet au lieu de Whisper

**Parakeet** (NVIDIA, 25 langues d'Europe dont le français) remplace **Whisper « small »**
d'OpenAI. Whisper traite toujours 30 secondes de son, même pour une phrase de deux secondes :
c'est sa façon d'être construit. Parakeet ne traite que ce qui a été dit, et d'un seul passage.

Sur les phrases d'essai, il est aussi **plus juste** : « 21 degrés » au lieu de « 20 et 1°C »,
« (H)erme les volets de la chambre » plutôt que « Hermelée volait de la chambre » (la voix de
synthèse des essais avale la première syllabe), et un point d'interrogation sur les questions (ce
qui aide [Assist](assist.md)). La mémoire reste la même (≈ 900 Mo).

`sudo ./installer.sh --voix` (ou la mise à jour de chaque nuit, qui garde l'option) télécharge
Parakeet (670 Mo, chaque fichier vérifié par son empreinte) et remplace Whisper.

### La voix : Piper gardé en marche

Avant, Piper était lancé pour chaque réponse : il relisait sa voix (63 Mo) à chaque fois, soit
0,6 s perdues. Maintenant il démarre avec le serveur et attend les phrases : on lui écrit la
phrase, il écrit le son dans un dossier à lui (`--output_dir`) et donne le nom du fichier. La même
voix sert aux réponses de la tablette et aux [annonces sur les enceintes](sonos.md).

Si Piper ne peut pas rester en marche (une version trop ancienne, un arrêt), la maison revient
toute seule à l'ancienne façon, une phrase à la fois, et réessaie une minute plus tard.

### La fin de phrase

La tablette considère la phrase finie après **0,9 s** de silence (au lieu de 1,2 s). Une courte
hésitation (« allume… la lampe ») passe encore. Le micro s'ouvre aussi un peu plus vite : le
traitement du son est préparé une fois, puis gardé en pause entre deux phrases (le micro, lui,
est bien fermé : le voyant du navigateur s'éteint).

### La tablette : ne redessiner que ce qui change

Trois causes, trois corrections :

1. **Les étiquettes du dessin de la maison** (« Salon 22,4° ») étaient replacées à chaque mesure,
   ce qui oblige le navigateur à recalculer toute la page aussitôt. Elles ne le sont plus que si
   leur texte change, ou si le dessin change de taille.
2. **Les blocs de la vue hub** (pièces, scènes, météo, agenda…) étaient reconstruits à chaque
   mesure, même identiques. Ils sont maintenant comparés avec ce qui est affiché, et laissés tels
   quels s'ils n'ont pas changé : le navigateur n'a rien à recalculer.
3. **Le tableau de bord caché sous la vue hub** était tenu à jour et mis en page alors qu'on ne le
   voyait pas. Il est mis de côté (`content-visibility: hidden`) et redessiné d'un coup en quittant
   la vue hub.

### L'ouverture : une adresse par version

`index.html` charge maintenant `/app.js?v=9f40f79eaf237fc5` : l'empreinte du fichier est dans
l'adresse. À cette adresse-là, le fichier ne changera jamais, donc le navigateur le garde sans
redemander (`Cache-Control: immutable`). Après une mise à jour du serveur, l'empreinte change,
l'adresse aussi, et le nouveau fichier arrive tout de suite. Seule la page est revérifiée à chaque
ouverture (300 octets).

### Au passage : l'arrêt du serveur

Le serveur attendait jusqu'à 90 s pour s'arrêter quand la boîte aux lettres des
[colis](colis.md) ne répondait pas. Le relevé en cours est maintenant abandonné aussitôt.

## Sécurité

- Parakeet est téléchargé à une version figée (Hugging Face, révision `2bda32ec`), chaque fichier
  **vérifié par son empreinte SHA-256** ; le service d'écoute garde tout son enfermement
  (fermé au réseau, compte jetable, 1,5 Go de mémoire au plus).
- Piper gardé en marche écrit dans **un dossier à lui** (droits `0700`, créé neuf à chaque
  démarrage, jamais réutilisé), vidé après chaque phrase et retiré à l'arrêt. La maison ne lit que
  des fichiers de ce dossier, quel que soit le chemin que Piper donne.
- Les adresses avec empreinte ne changent rien à ce qui est servi : les mêmes fichiers, avec la
  même [politique de sécurité](etape-6-interface-web.md) (CSP). Une ancienne empreinte est
  simplement revérifiée.
- La comparaison des blocs de la page se fait élément par élément (`isEqualNode`), jamais par du
  HTML : le tableau de bord n'utilise toujours aucun `innerHTML`.

## Comment c'est construit

| Fichier | Rôle |
| ------- | ---- |
| [crates/ha-tts/src/program.rs](../crates/ha-tts/src/program.rs) | Piper gardé en marche (`Resident`, `Slot`), la lecture du WAV, le retour à une phrase à la fois |
| [crates/ha-tts/src/lib.rs](../crates/ha-tts/src/lib.rs) | Une seule voix partagée, `warm_up` au démarrage |
| [crates/ha-stt/src/lib.rs](../crates/ha-stt/src/lib.rs) | Parakeet (avec `joiner.onnx`) ou Whisper, pour le programme lancé à chaque phrase |
| [packaging/linux/ha-maison-ecoute.service](../packaging/linux/ha-maison-ecoute.service) | Le service d'écoute, avec Parakeet |
| [packaging/linux/tout-compris.sh](../packaging/linux/tout-compris.sh) | `--voix` : Parakeet téléchargé et vérifié, Whisper remplacé |
| [crates/ha-api/src/web.rs](../crates/ha-api/src/web.rs) | Les adresses avec empreinte, gardées pour toujours |
| [web/utils.js](../crates/ha-api/web/utils.js) | `swap`, `setText`, `setAttr` : ne toucher la page que si ça change |
| [web/hub.js](../crates/ha-api/web/hub.js), [web/tiles.js](../crates/ha-api/web/tiles.js), [web/house.js](../crates/ha-api/web/house.js) | La vue hub, l'accueil et le dessin, redessinés seulement s'ils changent |
| [web/voix.js](../crates/ha-api/web/voix.js) | 0,9 s de silence, le traitement du son gardé en pause |
| [crates/ha-colis/src/client.rs](../crates/ha-colis/src/client.rs) | Le relevé abandonné à l'arrêt du serveur |

Essayé :
- l'écoute : Whisper et Parakeet sur les mêmes 26 phrases d'essai, côte à côte ; Parakeet avec 2
  et 4 fils ; puis le serveur complet, de la tablette à la réponse parlée ;
- Piper gardé en marche : démarré avec le serveur, trois phrases d'affilée sans le relancer, et un
  faux Piper trop ancien qui fait revenir à une phrase à la fois ;
- la tablette, dans Chromium avec le processeur ralenti 4 fois : la vue hub et le tableau complet
  sous deux mesures par seconde, les valeurs qui changent bien (« 3,5 kW », « 1 lumière » dans le
  salon), le tableau à jour en quittant la vue hub, aucune erreur, pas de fuite de mémoire ;
- l'ouverture : à la seconde ouverture, aucun fichier redemandé, seulement la page.

Pas encore essayé sur la vraie tablette ni sur le Ryzen du mini-PC.

### Concepts utilisés

**Le recalcul forcé** (*forced reflow*) : quand un script modifie la page puis demande aussitôt
la position d'un élément, le navigateur doit tout recalculer sur-le-champ pour répondre. Fait à
chaque mesure, c'est ce qui coûtait le plus. La solution : ne demander les positions que quand
quelque chose a bougé, ou laisser le navigateur prévenir (`ResizeObserver`) une fois la page
calculée.

**Le cache « immuable »** : une adresse dont le contenu ne change jamais peut être gardée par le
navigateur sans jamais redemander. Mettre l'empreinte du fichier dans l'adresse rend cela vrai :
un fichier modifié a une nouvelle adresse.

**Garder un programme en marche** : charger un modèle (la voix de Piper, le réseau de Parakeet)
coûte plus cher que de s'en servir. Le charger une fois et lui parler ensuite, c'est le principe
du service d'écoute depuis l'étape 84, appliqué maintenant à la voix.
