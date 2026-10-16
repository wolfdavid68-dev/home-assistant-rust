# Consignes projet — home-assistant-rust

## Pull requests

**Toujours ouvrir une PR en brouillon.** La CI ne tourne pas sur un brouillon
(voir l'en-tête de `.github/workflows/ci.yml`) : elle part au passage en
« prête », puis à chaque commit poussé. Le quota de minutes du dépôt privé est
la contrainte.

**Ne jamais passer une PR en « prête » sans l'accord explicite de David** :
c'est lui qui décide quand la CI tourne. Une fois le travail terminé et vérifié
en local, laisser la PR en brouillon et lui demander. Seulement après son
accord :

```bash
gh pr create --draft …
gh pr ready <numéro>   # uniquement après l'accord de David
```

Pour la même raison, ne pas pousser de commit sur une PR déjà « prête » sans
son accord, même un commit qui ne touche que de la documentation : sur une PR,
`paths-ignore` regarde tout le diff de la PR, pas le dernier commit, donc la CI
repart. Pour continuer le travail dans une PR « prête », la repasser d'abord en
brouillon (`POST /repos/{owner}/{repo}/pulls/{n}/ccr/convert_to_draft`).

Avant de demander le passage en « prête », lancer en local ce que la CI
vérifiera :

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Ajouter l'étiquette `windows` à une PR qui touche aux chemins de fichiers, au
port série ou aux permissions : c'est elle qui lance la vérification Windows.
