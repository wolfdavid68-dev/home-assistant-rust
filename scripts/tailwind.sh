#!/usr/bin/env sh
# Construit crates/ha-api/web/style.css depuis crates/ha-api/web/css/app.css avec
# l'outil autonome de Tailwind CSS (sans Node). Le télécharge au besoin dans
# target/, vérifié par son empreinte.
#   scripts/tailwind.sh           construit
#   scripts/tailwind.sh --check   échoue si style.css n'est pas à jour (CI)
set -eu
VERSION=4.3.5
cd "$(dirname "$0")/.."
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ASSET=tailwindcss-linux-x64 ;;
  Linux-aarch64) ASSET=tailwindcss-linux-arm64 ;;
  Darwin-arm64) ASSET=tailwindcss-macos-arm64 ;;
  Darwin-x86_64) ASSET=tailwindcss-macos-x64 ;;
  MINGW*|MSYS*|CYGWIN*) ASSET=tailwindcss-windows-x64.exe ;;
  *) echo "tailwind.sh : système non prévu" >&2; exit 1 ;;
esac
BIN="target/tailwind-$VERSION/$ASSET"
if [ ! -x "$BIN" ]; then
  mkdir -p "$(dirname "$BIN")"
  BASE="https://github.com/tailwindlabs/tailwindcss/releases/download/v$VERSION"
  curl -sSfL -o "$BIN" "$BASE/$ASSET"
  curl -sSfL -o "$BIN.sums" "$BASE/sha256sums.txt"
  WANT=$(awk -v a="./$ASSET" '$2 == a { print $1 }' "$BIN.sums")
  if command -v sha256sum >/dev/null; then GOT=$(sha256sum "$BIN" | cut -d' ' -f1)
  else GOT=$(shasum -a 256 "$BIN" | cut -d' ' -f1); fi
  rm -f "$BIN.sums"
  if [ -z "$WANT" ] || [ "$WANT" != "$GOT" ]; then
    echo "tailwind.sh : empreinte invalide" >&2; rm -f "$BIN"; exit 1
  fi
  chmod +x "$BIN"
fi
WEB=crates/ha-api/web
OUT="$WEB/style.css"
[ "${1:-}" = "--check" ] && OUT="$(mktemp)"
{
  echo "/* Fichier produit par scripts/tailwind.sh depuis css/app.css : ne pas modifier. */"
  "$BIN" -i "$WEB/css/app.css" -o - 2>/dev/null
} > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
if [ "${1:-}" = "--check" ]; then
  if ! cmp -s "$OUT" "$WEB/style.css"; then
    echo "style.css n'est pas à jour : lance scripts/tailwind.sh et ajoute-le au commit." >&2
    diff "$WEB/style.css" "$OUT" | head -20 >&2
    rm -f "$OUT"; exit 1
  fi
  rm -f "$OUT"
fi
