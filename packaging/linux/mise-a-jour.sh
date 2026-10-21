#!/bin/sh
# Mise à jour automatique du serveur de la maison (mini-PC sous Debian).
# Lancée chaque nuit par ha-maison-maj.timer. À la main :
#
#   sudo systemctl start ha-maison-maj      (puis : journalctl -u ha-maison-maj)
#
# Guide : docs/mini-pc.md. Dans l'ordre :
#   1. lit l'empreinte (SHA-256) de la dernière archive publiée : rien à
#      télécharger si c'est celle déjà vue ;
#   2. télécharge l'archive (HTTPS, depuis GitHub) et vérifie son empreinte ;
#   3. n'installe qu'une version plus récente que celle en place ;
#   4. sauvegarde la base, garde l'ancien programme, puis installe ;
#   5. si le nouveau serveur ne répond pas : remet l'ancien programme et la
#      base d'avant, et ne retente plus cette version.
set -eu

URL=${HA_MAJ_URL:-https://github.com/wolfdavid68-dev/home-assistant-rust/releases/latest/download}
ARCHIVE=ha-maison-linux-x86_64.tar.gz
ETAT=/var/lib/ha-maison-maj
OPT=/opt/ha-maison

die() {
  echo "erreur : $*" >&2
  exit 1
}

# Le numéro de version d'un programme ha-server (« ha-server 0.1.0 »).
version_de() {
  "$1" --version | awk '{ print $2 }'
}

# Tout le travail est dans une fonction : sh la lit en entier avant de
# commencer, et l'installation peut remplacer ce fichier sans gêner la suite.
main() {
  [ "$(id -u)" -eq 0 ] || die "à lancer en administrateur"
  command -v wget >/dev/null || die "wget manque : sudo apt install wget"
  mkdir -p "$ETAT"
  travail=$(mktemp -d)
  trap 'rm -rf "$travail"' EXIT
  cd "$travail"

  # 1. L'empreinte de la dernière archive publiée.
  if ! wget -q -O empreinte "$URL/$ARCHIVE.sha256"; then
    # Pas de réseau, ou pas encore de version publiée : pas une panne.
    echo "dernière version illisible ($URL) : nouvel essai la nuit prochaine"
    exit 0
  fi
  attendue=$(awk 'NR == 1 { print $1 }' empreinte)
  case "$attendue" in
    "" | *[!0-9a-f]*) die "empreinte illisible" ;;
  esac
  [ "${#attendue}" -eq 64 ] || die "empreinte illisible"
  if [ "$attendue" = "$(cat "$ETAT/vue" 2>/dev/null || true)" ]; then
    echo "déjà à jour"
    exit 0
  fi
  if [ "$attendue" = "$(cat "$ETAT/refusee" 2>/dev/null || true)" ]; then
    echo "cette version a déjà échoué ici : en attente de la suivante"
    exit 0
  fi

  # 2. L'archive, vérifiée avant d'en lire quoi que ce soit.
  wget -q -O "$ARCHIVE" "$URL/$ARCHIVE" || die "téléchargement impossible"
  echo "$attendue  $ARCHIVE" | sha256sum -c --quiet - ||
    die "l'archive ne correspond pas à son empreinte : rien n'est installé"
  tar -xzf "$ARCHIVE"
  set -- ha-maison-v*-linux-x86_64
  if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
    die "archive inattendue"
  fi
  dossier=$PWD/$1

  # 3. Seulement une version plus récente (jamais de retour en arrière).
  nouvelle=$(version_de "$dossier/ha-server") || die "le nouveau programme ne démarre pas ici"
  actuelle=$(version_de "$OPT/ha-server")
  plus_recente=$(printf '%s\n%s\n' "$nouvelle" "$actuelle" | sort -V | tail -n 1)
  if [ "$nouvelle" = "$actuelle" ] || [ "$plus_recente" != "$nouvelle" ]; then
    echo "version publiée $nouvelle, installée $actuelle : rien à faire"
    echo "$attendue" >"$ETAT/vue"
    exit 0
  fi
  echo "mise à jour : $actuelle -> $nouvelle"

  # 4. La base d'abord, puis de quoi revenir en arrière.
  sortie=$(/usr/local/bin/ha-maison backup now) || die "sauvegarde impossible : rien n'est installé"
  sauvegarde=$(printf '%s\n' "$sortie" | sed -n 's/^sauvegarde vérifiée : \(.*\) ([0-9]* Kio)$/\1/p')
  [ -n "$sauvegarde" ] || die "sauvegarde introuvable : rien n'est installé"
  echo "base sauvegardée : $sauvegarde"
  cp -p "$OPT/ha-server" "$ETAT/ha-server.precedent"
  cp -p "$OPT/mise-a-jour.sh" "$ETAT/mise-a-jour.sh.precedent"
  cp -p /usr/local/bin/ha-maison "$ETAT/ha-maison.precedent"
  cp -p /etc/systemd/system/ha-maison.service "$ETAT/ha-maison.service.precedent"

  if "$dossier/installer.sh"; then
    echo "$attendue" >"$ETAT/vue"
    echo "mise à jour terminée : $nouvelle"
    exit 0
  fi

  # 5. Le nouveau serveur ne répond pas : tout remettre comme avant.
  echo "la version $nouvelle ne démarre pas : retour à $actuelle" >&2
  systemctl stop ha-maison.service || true
  install -m 0755 "$ETAT/ha-server.precedent" "$OPT/ha-server"
  install -m 0755 "$ETAT/mise-a-jour.sh.precedent" "$OPT/mise-a-jour.sh"
  install -m 0755 "$ETAT/ha-maison.precedent" /usr/local/bin/ha-maison
  install -m 0644 "$ETAT/ha-maison.service.precedent" /etc/systemd/system/ha-maison.service
  systemctl daemon-reload
  # La nouvelle version a pu transformer la base : on reprend celle d'avant.
  /usr/local/bin/ha-maison backup restore "$sauvegarde" --force
  systemctl start ha-maison.service
  echo "$attendue" >"$ETAT/refusee"
  die "version $nouvelle refusée ; $actuelle remise en place avec la base de $sauvegarde"
}

main "$@"
