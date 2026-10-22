#!/bin/sh
# Installe, ou met à jour, le serveur de la maison sur un mini-PC sous
# Debian. À lancer depuis le dossier décompressé :
#
#   sudo ./installer.sh                   le serveur, et tout ce qu'il lui faut
#   sudo ./installer.sh --sonos --ring    en plus : la voix des annonces sur les
#                                         enceintes Sonos, et la sonnette Ring
#
# Les options sont retenues (/etc/ha-maison/options) : la mise à jour de
# chaque nuit les garde. Guide : docs/mini-pc.md
#
# Ce que fait le script (et rien d'autre) :
#   - un compte système « ha-maison », sans mot de passe ni shell ;
#   - le programme dans /opt/ha-maison, la commande « ha-maison » ;
#   - les réglages dans /etc/ha-maison (jamais remplacés ensuite) ;
#   - les mises à jour de sécurité de Debian, automatiques ;
#   - le facteur MQTT (Mosquitto), pour les programmes du mini-PC seulement,
#     avec des mots de passe tirés au hasard (étape 83) ;
#   - avec --sonos : la voix (Piper), téléchargée et vérifiée ;
#   - avec --ring : ring-mqtt, dans un conteneur à la version épinglée ;
#   - le service systemd, démarré tout de suite et à chaque allumage ;
#   - la mise à jour automatique, chaque nuit (mise-a-jour.sh).
# Les données restent dans /var/lib/ha-maison* : une mise à jour n'y touche pas.
set -eu

cd "$(dirname "$0")"

die() {
  echo "erreur : $*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || die "à lancer avec sudo : sudo ./installer.sh"
[ -d /run/systemd/system ] || die "systemd n'est pas actif sur cette machine"
[ -f ./ha-server ] || die "ha-server introuvable : lance le script depuis le dossier décompressé"
case "$(uname -m)" in
  x86_64) ;;
  *) die "ce programme est fait pour un PC 64 bits (x86_64), pas pour $(uname -m)" ;;
esac
version=$(./ha-server --version) || die "ha-server ne démarre pas sur cette machine"

# Les options : celles d'avant (la mise à jour de chaque nuit n'en donne
# aucune), plus celles données maintenant.
OPTIONS=/etc/ha-maison/options
avec_sonos=false
avec_ring=false
if grep -qx sonos "$OPTIONS" 2>/dev/null; then avec_sonos=true; fi
if grep -qx ring "$OPTIONS" 2>/dev/null; then avec_ring=true; fi
for option in "$@"; do
  case "$option" in
    --sonos) avec_sonos=true ;;
    --ring) avec_ring=true ;;
    *) die "option inconnue : $option (possibles : --sonos, --ring)" ;;
  esac
done
echo "Installation de $version"

# 1. Le compte du service.
if ! id ha-maison >/dev/null 2>&1; then
  useradd --system --user-group --no-create-home \
    --home-dir /var/lib/ha-maison --shell /usr/sbin/nologin ha-maison
  echo "  compte système « ha-maison » créé"
fi

# 2. Le programme. Copié à côté, puis mis en place d'un coup : le serveur en
# marche garde l'ancien jusqu'au redémarrage.
install -d -m 0755 /opt/ha-maison
install -m 0755 ha-server /opt/ha-maison/ha-server.nouveau
mv -f /opt/ha-maison/ha-server.nouveau /opt/ha-maison/ha-server
install -m 0755 mise-a-jour.sh /opt/ha-maison/mise-a-jour.sh
install -m 0755 ha-maison /usr/local/bin/ha-maison

# 3. Les réglages : lisibles par le service, modifiables seulement avec sudo.
premiere_fois=false
install -d -m 0750 -g ha-maison /etc/ha-maison /etc/ha-maison/secrets
if [ ! -e /etc/ha-maison/ha-maison.env ]; then
  install -m 0640 -g ha-maison ha-maison.env /etc/ha-maison/ha-maison.env
  premiere_fois=true
fi
# Les exemples en 0644 : une copie (« sudo cp areas.example.yaml areas.yaml »)
# reste lisible par le service. Le dossier, lui, est fermé aux autres comptes.
for exemple in ./*.example.yaml; do
  [ -e "$exemple" ] || continue
  install -m 0644 "$exemple" /etc/ha-maison/
done

{
  echo "# Les options de l'installation (installer.sh), gardées par les mises à jour."
  if [ "$avec_sonos" = true ]; then echo sonos; fi
  if [ "$avec_ring" = true ]; then echo ring; fi
} >"$OPTIONS"
chmod 0644 "$OPTIONS"

# 3 bis. Tout ce qu'il faut autour du serveur (étape 83).
# shellcheck disable=SC1091 # à côté de ce script, dans l'archive
. ./tout-compris.sh
tout_compris

# 4. Le service, et sa mise à jour de chaque nuit. Celle-ci n'est activée
# qu'à la première installation : si tu l'as arrêtée, elle le reste.
maj_nouvelle=false
[ -e /etc/systemd/system/ha-maison-maj.timer ] || maj_nouvelle=true
install -m 0644 ha-maison.service ha-maison-maj.service ha-maison-maj.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable ha-maison.service >/dev/null 2>&1
if [ "$maj_nouvelle" = true ]; then
  systemctl enable --now ha-maison-maj.timer >/dev/null 2>&1
fi
systemctl restart ha-maison.service

printf "  démarrage"
pret=false
for _ in $(seq 1 30); do
  if /usr/local/bin/ha-maison healthcheck >/dev/null 2>&1; then
    pret=true
    break
  fi
  printf "."
  sleep 1
done
echo

if [ "$pret" != true ]; then
  journalctl -u ha-maison.service -n 20 --no-pager || true
  die "le serveur ne répond pas ; voir « sudo journalctl -u ha-maison »"
fi

adresse=$(hostname -I 2>/dev/null | awk '{ print $1 }')
adresse=${adresse:-<adresse du mini-PC>}
echo "Le serveur tourne, et redémarrera tout seul à chaque allumage."
if systemctl is-enabled --quiet ha-maison-maj.timer; then
  echo "Il se met à jour tout seul, chaque nuit vers 4 h."
fi
if grep -q '^HA_TLS=on' /etc/ha-maison/ha-maison.env; then
  echo "Tableau de bord : https://${adresse}:8123/"
  echo "Sur chaque appareil, installe d'abord le certificat de la maison :"
  echo "  http://${adresse}:8123/maison-ca.crt   (voir docs/https-maison.md)"
else
  echo "Tableau de bord : http://${adresse}:8123/"
fi
if [ "$avec_ring" = true ] && [ ! -s /var/lib/ring-mqtt/ring-state.json ]; then
  echo
  echo "Sonnette Ring : sur ton téléphone, ouvre http://${adresse}:55123/"
  echo "et connecte le compte Ring (le code arrive par SMS). Une seule fois."
fi
if [ "$premiere_fois" = true ]; then
  echo
  echo "Crée maintenant ton compte (le mot de passe est demandé deux fois) :"
  echo "  sudo ha-maison user add <ton nom>"
  echo "Les réglages sont dans /etc/ha-maison/ha-maison.env."
fi
