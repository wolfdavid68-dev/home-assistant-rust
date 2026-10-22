# shellcheck shell=sh
# Tout ce qu'il faut autour du serveur de la maison (étape 83), comme
# Home Assistant OS : lu par installer.sh, qui appelle « tout_compris ».
#
#   - les mises à jour de sécurité de Debian, automatiques ;
#   - Mosquitto, le facteur MQTT, pour les programmes du mini-PC seulement
#     (127.0.0.1), un compte par programme, mots de passe tirés au hasard ;
#   - avec --sonos : Piper et la voix française, vérifiés par leur empreinte ;
#   - avec --ring : ring-mqtt, dans un conteneur épinglé (ha-maison-ring.service) ;
#   - avec --voix : parler à la maison depuis la tablette (étape 84) : Whisper
#     et sherpa-onnx, vérifiés par leur empreinte (ha-maison-ecoute.service),
#     et Piper pour répondre.
#
# Les réglages produits vont dans /etc/ha-maison/ha-maison.auto.env : les tiens
# (ha-maison.env) passent devant. Chaque étape est sautée si elle est déjà
# faite : la mise à jour de chaque nuit relance ce script sans rien casser.

# Les options, données par installer.sh (non : par défaut).
: "${avec_sonos:=false}" "${avec_ring:=false}" "${avec_voix:=false}"

# Piper (la voix) : version et empreintes vérifiées.
PIPER_URL=https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz
PIPER_SHA256=a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992
VOIX_URL=https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/fr/fr_FR/siwis/medium
VOIX_ONNX_SHA256=641d1ab097da2b81128c076810edb052b385decc8be3381814802a64a73baf99
VOIX_JSON_SHA256=39479916c2db192b5ac9764daddd0c744d83e023ad890c6976c0633ae4df8959

# L'écoute (étape 84) : sherpa-onnx 1.13.8 (le programme et sa bibliothèque,
# pris dans ses paquets Python officiels) et Whisper « small » en entiers
# 8 bits, à une version figée.
SHERPA_BIN_URL=https://files.pythonhosted.org/packages/py3/s/sherpa-onnx-bin/sherpa_onnx_bin-1.13.8-py3-none-manylinux2014_x86_64.whl
SHERPA_BIN_SHA256=cb835b6fd48a1d74809bf327b01515ec94c93428203c858885e9b09f3fe63227
SHERPA_CORE_URL=https://files.pythonhosted.org/packages/py3/s/sherpa-onnx-core/sherpa_onnx_core-1.13.8-py3-none-manylinux2014_x86_64.whl
SHERPA_CORE_SHA256=4da90acf435373d7b2ba9cc0be806e7e274d28f63f5780880b5dea6721626e36
WHISPER_URL=https://huggingface.co/csukuangfj/sherpa-onnx-whisper-small/resolve/8f3c18b358db4d1f2fc1eae49d75cd20989e4309
WHISPER_ENCODER_SHA256=4cbe7b22fa9026b843b60a68640c747de05bafb1a11b57edc0e66c232d9f33a9
WHISPER_DECODER_SHA256=acad50b5c782696e91b55914cc5ab4f756f1532f76e22aa6fc615f39fb69a8ee
WHISPER_TOKENS_SHA256=b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126
ECOUTE=/opt/ha-maison/ecoute

MOSQUITTO_CONF=/etc/mosquitto/conf.d/ha-maison.conf
MOSQUITTO_PASSWD=/etc/mosquitto/ha-maison.passwd
SECRETS=/etc/ha-maison/secrets
AUTO_ENV=/etc/ha-maison/ha-maison.auto.env

# Installe des paquets Debian qui manquent (sans les « recommandés »).
paquets() {
  manque=""
  for paquet in "$@"; do
    dpkg-query -W -f='${Status}' "$paquet" 2>/dev/null | grep -q "ok installed" ||
      manque="$manque $paquet"
  done
  [ -n "$manque" ] || return 0
  echo "  installation de :$manque"
  # shellcheck disable=SC2086 # une liste de paquets, voulue
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends $manque >/dev/null ||
    die "installation impossible de :$manque (réseau ? sudo apt update ?)"
}

# Un mot de passe tiré au hasard : 32 lettres et chiffres.
mot_de_passe() {
  tr -dc 'A-Za-z0-9' </dev/urandom | head -c 32
}

# Un réglage est-il déjà dans tes réglages à toi (ligne active) ?
regle_par_toi() {
  grep -q "^$1=" /etc/ha-maison/ha-maison.env 2>/dev/null
}

# Télécharge $1 dans $2 et vérifie son empreinte $3.
telecharge() {
  wget -q -O "$2" "$1" || die "téléchargement impossible : $1"
  echo "$3  $2" | sha256sum -c --quiet - || die "empreinte fausse pour $1 : rien n'est installé"
}

mises_a_jour_debian() {
  paquets unattended-upgrades
  if [ ! -e /etc/apt/apt.conf.d/20auto-upgrades ]; then
    cat >/etc/apt/apt.conf.d/20auto-upgrades <<'FIN'
// Écrit par le serveur de la maison (installer.sh) : chaque jour, les
// correctifs de sécurité de Debian, installés tout seuls.
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
FIN
    echo "  mises à jour de sécurité de Debian : automatiques"
  fi
}

# Un compte Mosquitto : son mot de passe est rangé dans le fichier $2.
compte_mqtt() {
  if [ -s "$2" ] && grep -q "^$1:" "$MOSQUITTO_PASSWD" 2>/dev/null; then
    return 0
  fi
  secret=$(mot_de_passe)
  [ "${#secret}" -eq 32 ] || die "mot de passe impossible à tirer"
  umask 077
  printf '%s' "$secret" >"$2"
  umask 022
  touch "$MOSQUITTO_PASSWD"
  mosquitto_passwd -b "$MOSQUITTO_PASSWD" "$1" "$secret" 2>/dev/null ||
    die "compte MQTT « $1 » impossible à créer"
  mosquitto_change=true
  echo "  compte MQTT « $1 » créé (mot de passe dans $2)"
}

mosquitto_local() {
  # Tu as ton propre broker (HA_MQTT_HOST dans ha-maison.env) : on n'y touche pas.
  if regle_par_toi HA_MQTT_HOST; then
    echo "  MQTT : ton broker (HA_MQTT_HOST) est gardé"
    return 0
  fi
  paquets mosquitto
  mosquitto_change=false
  install -d -m 0755 /etc/mosquitto/conf.d
  conf=$(cat <<'FIN'
# Écrit par le serveur de la maison (installer.sh, étape 83). Le facteur MQTT
# pour les programmes du mini-PC seulement (le serveur, ring-mqtt). Pour des
# appareils sur le réseau (Zigbee2MQTT…), ajoute ton propre fichier à côté.
listener 1883 127.0.0.1
allow_anonymous false
password_file /etc/mosquitto/ha-maison.passwd
FIN
)
  if [ "$(cat "$MOSQUITTO_CONF" 2>/dev/null)" != "$conf" ]; then
    printf '%s\n' "$conf" >"$MOSQUITTO_CONF"
    mosquitto_change=true
  fi
  compte_mqtt ha-server "$SECRETS/mqtt_password"
  chgrp ha-maison "$SECRETS/mqtt_password"
  chmod 0640 "$SECRETS/mqtt_password"
  if [ "$avec_ring" = true ]; then
    compte_mqtt ring "$SECRETS/mqtt_ring_password"
    chmod 0600 "$SECRETS/mqtt_ring_password"
  fi
  chown root:mosquitto "$MOSQUITTO_PASSWD"
  chmod 0640 "$MOSQUITTO_PASSWD"
  systemctl enable mosquitto >/dev/null 2>&1
  if [ "$mosquitto_change" = true ]; then
    systemctl restart mosquitto
  else
    systemctl start mosquitto
  fi
}

voix() {
  # La voix sert aux annonces (--sonos) et aux réponses à la tablette (--voix).
  [ "$avec_sonos" = true ] || [ "$avec_voix" = true ] || return 0
  if [ -x /opt/ha-maison/piper/piper ] &&
    echo "$VOIX_ONNX_SHA256  /opt/ha-maison/piper/fr_FR-siwis-medium.onnx" | sha256sum -c --quiet - >/dev/null 2>&1; then
    return 0
  fi
  echo "  la voix de la maison (Piper, 90 Mo)…"
  paquets wget ca-certificates
  dossier=$(mktemp -d)
  telecharge "$PIPER_URL" "$dossier/piper.tar.gz" "$PIPER_SHA256"
  telecharge "$VOIX_URL/fr_FR-siwis-medium.onnx" "$dossier/fr_FR-siwis-medium.onnx" "$VOIX_ONNX_SHA256"
  telecharge "$VOIX_URL/fr_FR-siwis-medium.onnx.json" "$dossier/fr_FR-siwis-medium.onnx.json" "$VOIX_JSON_SHA256"
  rm -rf /opt/ha-maison/piper
  tar -xzf "$dossier/piper.tar.gz" -C /opt/ha-maison --no-same-owner
  install -m 0644 "$dossier/fr_FR-siwis-medium.onnx" "$dossier/fr_FR-siwis-medium.onnx.json" /opt/ha-maison/piper/
  rm -rf "$dossier"
  [ -x /opt/ha-maison/piper/piper ] || die "Piper mal installé"
  echo "  voix française installée"
}

ring() {
  if [ "$avec_ring" != true ]; then
    # Retirée des options : on l'arrête (sans effacer son jeton).
    if [ -e /etc/systemd/system/ha-maison-ring.service ]; then
      systemctl disable --now ha-maison-ring.service >/dev/null 2>&1 || true
    fi
    return 0
  fi
  if regle_par_toi HA_MQTT_HOST; then
    die "--ring : ring-mqtt passe par le Mosquitto du mini-PC, retire HA_MQTT_HOST de ha-maison.env"
  fi
  paquets docker.io
  # Debian 13 range la commande « docker » à part (docker-cli, seulement
  # « recommandé ») ; Debian 12 la met dans docker.io.
  command -v docker >/dev/null || paquets docker-cli
  systemctl enable --now docker >/dev/null 2>&1
  install -d -m 0700 /var/lib/ring-mqtt
  if [ ! -e /var/lib/ring-mqtt/config.json ]; then
    umask 077
    cat >/var/lib/ring-mqtt/config.json <<FIN
{
  "mqtt_url": "mqtt://ring:$(cat "$SECRETS/mqtt_ring_password")@127.0.0.1:1883",
  "mqtt_options": "",
  "livestream_user": "",
  "livestream_pass": "",
  "disarm_code": "",
  "enable_cameras": true,
  "enable_modes": false,
  "enable_panic": false,
  "hass_topic": "homeassistant/status",
  "ring_topic": "ring",
  "location_ids": []
}
FIN
    umask 022
    echo "  ring-mqtt réglé"
  fi
  # L'image d'abord (seulement si elle n'est pas déjà là) : un téléchargement
  # raté laisse l'ancien service en place, et sera retenté la fois suivante.
  image=$(sed -n 's/^Environment=IMAGE=//p' ha-maison-ring.service)
  [ -n "$image" ] || die "ha-maison-ring.service : image introuvable"
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    echo "  ring-mqtt : téléchargement de $(printf '%s' "$image" | sed 's/@.*//; s/.*\///')…"
    docker pull -q "$image" >/dev/null || die "image ring-mqtt introuvable : $image"
  fi
  ancien=$(cat /etc/systemd/system/ha-maison-ring.service 2>/dev/null || true)
  install -m 0644 ha-maison-ring.service /etc/systemd/system/
  systemctl daemon-reload
  systemctl enable ha-maison-ring.service >/dev/null 2>&1
  if [ "$ancien" != "$(cat ha-maison-ring.service)" ]; then
    systemctl restart ha-maison-ring.service
    # Les anciennes versions de ring-mqtt ne servent plus.
    garder=$(docker image inspect -f '{{.Id}}' "$image" 2>/dev/null || true)
    for vieille in $(docker images --no-trunc -q tsightler/ring-mqtt 2>/dev/null | sort -u); do
      [ "$vieille" = "$garder" ] || docker rmi "$vieille" >/dev/null 2>&1 || true
    done
  else
    systemctl start ha-maison-ring.service
  fi
}

# L'écoute (étape 84) : Whisper, gardé en mémoire par un service à part.
ecoute() {
  if [ "$avec_voix" != true ]; then
    # Retirée des options : on l'arrête (les fichiers restent, 400 Mo).
    if [ -e /etc/systemd/system/ha-maison-ecoute.service ]; then
      systemctl disable --now ha-maison-ecoute.service >/dev/null 2>&1 || true
    fi
    return 0
  fi
  memoire=$(awk '/^MemTotal:/ { print int($2 / 1024) }' /proc/meminfo)
  if [ "${memoire:-0}" -lt 3000 ]; then
    echo "  attention : $memoire Mo de mémoire ; l'écoute en prend 800 à elle seule"
  fi
  fichiers_neufs=false
  if ! echo "$WHISPER_ENCODER_SHA256  $ECOUTE/encoder.onnx" | sha256sum -c --quiet - >/dev/null 2>&1 ||
    ! echo "$WHISPER_DECODER_SHA256  $ECOUTE/decoder.onnx" | sha256sum -c --quiet - >/dev/null 2>&1 ||
    [ ! -x "$ECOUTE/sherpa-onnx-offline-websocket-server" ]; then
    echo "  l'écoute (Whisper et sherpa-onnx, 400 Mo)…"
    paquets wget ca-certificates unzip
    dossier=$(mktemp -d)
    telecharge "$SHERPA_BIN_URL" "$dossier/bin.whl" "$SHERPA_BIN_SHA256"
    telecharge "$SHERPA_CORE_URL" "$dossier/core.whl" "$SHERPA_CORE_SHA256"
    telecharge "$WHISPER_URL/small-encoder.int8.onnx" "$dossier/encoder.onnx" "$WHISPER_ENCODER_SHA256"
    telecharge "$WHISPER_URL/small-decoder.int8.onnx" "$dossier/decoder.onnx" "$WHISPER_DECODER_SHA256"
    telecharge "$WHISPER_URL/small-tokens.txt" "$dossier/tokens.txt" "$WHISPER_TOKENS_SHA256"
    unzip -q -j -o "$dossier/bin.whl" \
      'sherpa_onnx_bin-1.13.8.data/data/bin/sherpa-onnx-offline-websocket-server' -d "$dossier" ||
      die "paquet sherpa-onnx illisible"
    unzip -q -j -o "$dossier/core.whl" 'sherpa_onnx/lib/libonnxruntime.so' -d "$dossier" ||
      die "paquet sherpa-onnx illisible"
    rm -rf "$ECOUTE.nouveau"
    install -d -m 0755 "$ECOUTE.nouveau"
    install -m 0755 "$dossier/sherpa-onnx-offline-websocket-server" "$ECOUTE.nouveau/"
    install -m 0644 "$dossier/libonnxruntime.so" "$dossier/encoder.onnx" \
      "$dossier/decoder.onnx" "$dossier/tokens.txt" "$ECOUTE.nouveau/"
    rm -rf "$dossier" "$ECOUTE"
    mv "$ECOUTE.nouveau" "$ECOUTE"
    fichiers_neufs=true
    echo "  écoute installée"
  fi
  # Le nombre de processeurs pour Whisper : tous, jusqu'à 4.
  fils=$(nproc 2>/dev/null || echo 2)
  [ "$fils" -le 4 ] || fils=4
  printf 'ECOUTE_FILS=%s\n' "$fils" >/etc/ha-maison/ecoute.env
  chmod 0644 /etc/ha-maison/ecoute.env
  ancien=$(cat /etc/systemd/system/ha-maison-ecoute.service 2>/dev/null || true)
  install -m 0644 ha-maison-ecoute.service /etc/systemd/system/
  systemctl daemon-reload
  systemctl enable ha-maison-ecoute.service >/dev/null 2>&1
  # Redémarré seulement s'il a changé : il garde Whisper en mémoire.
  if [ "$fichiers_neufs" = true ] || [ "$ancien" != "$(cat ha-maison-ecoute.service)" ]; then
    systemctl restart ha-maison-ecoute.service
  else
    systemctl start ha-maison-ecoute.service
  fi
}

# Les réglages produits ici, pour le serveur.
reglages_auto() {
  {
    echo "# Écrit par installer.sh (étape 83) : ne pas modifier, il est réécrit."
    echo "# Tes réglages (ha-maison.env) passent devant ceux-ci."
    if ! regle_par_toi HA_MQTT_HOST; then
      echo "HA_MQTT_HOST=127.0.0.1"
      echo "HA_MQTT_USER=ha-server"
      echo "HA_MQTT_PASSWORD_FILE=$SECRETS/mqtt_password"
    fi
    if [ "$avec_sonos" = true ]; then
      echo "HA_SONOS=auto"
    fi
    if [ "$avec_sonos" = true ] || [ "$avec_voix" = true ]; then
      echo "HA_PIPER=/opt/ha-maison/piper/piper"
    fi
    if [ "$avec_voix" = true ]; then
      echo "HA_ECOUTE=ws://127.0.0.1:10350"
    fi
  } >"$AUTO_ENV.nouveau"
  chgrp ha-maison "$AUTO_ENV.nouveau"
  chmod 0640 "$AUTO_ENV.nouveau"
  mv -f "$AUTO_ENV.nouveau" "$AUTO_ENV"
}

tout_compris() {
  if [ "${HA_SANS_PAQUETS:-}" = 1 ]; then
    echo "  (HA_SANS_PAQUETS=1 : ni Mosquitto, ni voix, ni écoute, ni ring-mqtt)"
    return 0
  fi
  command -v apt-get >/dev/null || die "apt-get introuvable : ce script est fait pour Debian"
  # La liste des paquets, une fois par jour au plus.
  if [ -z "$(find /var/lib/apt/lists -maxdepth 1 -name '*Packages*' -mtime -1 2>/dev/null | head -n 1)" ]; then
    DEBIAN_FRONTEND=noninteractive apt-get update -q >/dev/null || echo "  (liste des paquets Debian non rafraîchie)"
  fi
  mises_a_jour_debian
  mosquitto_local
  voix
  ecoute
  ring
  reglages_auto
}
