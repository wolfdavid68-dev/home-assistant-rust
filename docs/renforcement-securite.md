# Renforcement de la sécurité, expliqué

Trois améliorations, dans l'ordre :

1. **les jetons des appareils chiffrés dans la base** (ce document) ;
2. la double authentification (à venir) ;
3. un journal d'audit (à venir).

## 1. Les jetons des appareils, chiffrés

### Le problème

Pour piloter tes lampes et tes volets, le serveur garde deux secrets dans `ha.db` : la **clé
d'application** du pont Hue et le **jeton** de la box Somfy. Il doit pouvoir les relire à chaque
démarrage, donc on ne peut pas les « hacher » comme les mots de passe.

Jusqu'ici, ils étaient en clair. Une copie de `ha.db` (une sauvegarde oubliée sur une clé USB,
un disque revendu…) suffisait pour piloter ta maison depuis ton réseau.

### La solution

Les secrets sont maintenant **chiffrés**, avec une clé rangée dans **un autre fichier** :

```
ha.db   ──  enc:v1:8f3a…c41e   (jeton Somfy chiffré : illisible)
ha.key  ──  la clé (64 caractères), lisible par toi seul
```

- Sans Docker, `ha.key` est créé à côté de `ha.db` au premier démarrage. La variable
  `HA_KEY_PATH` permet de le mettre ailleurs (sur une clé USB, par exemple).
- Avec Docker, la clé est dans son propre volume (`ha-cle`). L'outil de sauvegarde ne copie
  que la base : **une sauvegarde volée ne permet pas de piloter tes appareils**.
- Les associations faites avant cette mise à jour sont **chiffrées automatiquement** au
  démarrage suivant (le journal affiche « jeton de la box Somfy chiffré dans la base »).
- **Si la clé est perdue**, le serveur démarre quand même. Il signale l'association illisible
  et demande de refaire `hue pair` ou `somfy pair`. Le reste de la maison continue de fonctionner.

Mets la clé de côté une fois (gestionnaire de mots de passe) : la commande est dans le
[guide de l'étape 7](etape-7-deploiement.md#sauvegarder-et-restaurer).

### Ce que ça protège, et ce que ça ne protège pas

| Situation | Protégé ? |
|---|---|
| Quelqu'un récupère une sauvegarde de `ha.db` | **Oui** : les jetons sont illisibles |
| Quelqu'un modifie un jeton chiffré dans la base | **Oui** : la modification est détectée, le jeton est refusé |
| Quelqu'un recopie le jeton Somfy à la place de la clé Hue | **Oui** : chaque secret est lié à son usage |
| Quelqu'un a accès à toute la machine (base **et** clé) | **Non** : aucun chiffrement ne protège de ça. C'est le rôle des droits du système, de Docker et du chiffrement du disque (BitLocker sous Windows) |

### L'algorithme

**XChaCha20-Poly1305**, de la bibliothèque RustCrypto (Rust pur, sans `unsafe` de notre côté) :

- c'est un chiffrement **authentifié** : en plus de cacher le contenu, il détecte toute
  modification ;
- chaque secret a son propre **nonce** aléatoire de 24 octets. Le même jeton chiffré deux fois
  donne deux résultats différents ;
- le **contexte** (`"hue_bridge.app_key"`, `"somfy_box.token"`) est ajouté comme « donnée
  associée » : il n'est pas chiffré, mais il est vérifié au déchiffrement.

### Concepts Rust rencontrés

- **`Arc` pour partager sans copier.** Le `Store` est cloné partout (enregistreur, API, Hue,
  Somfy). La clé est dans un `Arc` : tous les clones pointent vers la même clé en mémoire, au
  lieu d'en faire des copies.
- **Effacer la mémoire (`zeroize`).** La clé et les secrets déchiffrés sont dans des
  `Zeroizing<…>` : ils sont remplis de zéros quand on n'en a plus besoin, au lieu de traîner en
  mémoire. Le chiffreur lui-même efface sa clé (option `zeroize` de la crate).
- **Un `Debug` écrit à la main.** `#[derive(Debug)]` afficherait la clé dans un message de
  débogage. On écrit donc `impl Debug` nous-mêmes, pour afficher `SecretKey(<masquée>)`.
- **`split_first_chunk::<24>()`.** Cette méthode coupe une tranche en « les 24 premiers octets »
  (un tableau de taille connue à la compilation) et « le reste ». Elle renvoie `None` si la
  tranche est trop courte : pas d'indice qui déborde, pas de `panic`.

### Testé

- Chiffrer puis déchiffrer redonne le secret ; deux chiffrements du même secret diffèrent.
- Mauvaise clé, mauvais contexte, secret modifié ou tronqué : tous refusés.
- Le fichier de clé est créé avec les droits 600, puis réutilisé ; un fichier de clé abîmé
  est refusé avec un message clair.
- Base relue après redémarrage : le secret se déchiffre. Base **sans sa clé** : impossible.
- Hue et Somfy : la valeur en base ne contient plus le jeton, et une ancienne association en
  clair est rechiffrée à la lecture.
- Clé perdue : le serveur démarre, signale l'association illisible et passe aux appareils
  restants.
- Docker : clé créée dans `/cle/ha.key` (droits 600, utilisateur sans privilège), conteneur
  « healthy ».
