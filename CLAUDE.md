# Consignes projet — home-assistant-rust

## Pull requests

**Toujours ouvrir une PR en brouillon**, puis la passer en « prête » une fois
le travail terminé et vérifié en local :

```bash
gh pr create --draft …
gh pr ready <numéro>
```

La CI ne tourne pas sur un brouillon (voir l'en-tête de `.github/workflows/ci.yml`).
Une PR ouverte directement en « prête » relance toute la CI à chaque commit
poussé, alors que seul le dernier verdict compte. Le quota de minutes du dépôt
privé est la contrainte.

Avant de passer en « prête », lancer en local ce que la CI vérifiera :

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Ajouter l'étiquette `windows` à une PR qui touche aux chemins de fichiers, au
port série ou aux permissions : c'est elle qui lance la vérification Windows.
