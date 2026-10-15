# syntax=docker/dockerfile:1
#
# Image Docker de ha-server, en deux étapes :
# 1. « builder » : une image Rust complète compile le programme ;
# 2. l'image finale ne contient que le programme et le strict nécessaire
#    (bibliothèque C, certificats, fuseaux horaires) : ni shell, ni
#    gestionnaire de paquets, ni compilateur. Moins il y a d'outils dans
#    l'image, moins un attaquant a de prise.
#
# Les images de base sont épinglées par leur empreinte (sha256) : une image
# modifiée en douce sur le registre ne serait pas utilisée. Dependabot
# propose les mises à jour.
#
# Construire : docker build -t ha-server .
# (Le plus simple est de passer par compose.yaml : docker compose up -d --build)

FROM rust:1.99-slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa AS builder

WORKDIR /src
COPY . .
# On compile avec le Rust de l'image (version épinglée ci-dessus), sans
# télécharger la version « stable » du moment demandée par ce fichier.
RUN rm rust-toolchain.toml

# Les caches (« --mount=type=cache ») évitent de tout retélécharger et
# recompiler à chaque construction. `--locked` : exactement les versions
# de Cargo.lock, celles vérifiées par cargo deny.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ha-server \
    && cp target/release/ha-server /usr/local/bin/ha-server \
    && mkdir /data-empty

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

COPY --from=builder /usr/local/bin/ha-server /usr/local/bin/ha-server
# Le dossier des données appartient à l'utilisateur sans privilège
# « nonroot » (n° 65532) sous lequel tourne le programme.
COPY --from=builder --chown=65532:65532 /data-empty /data
# La clé qui chiffre les secrets de la base est rangée à part : une
# sauvegarde de /data seule ne suffit pas à lire les jetons des appareils.
COPY --from=builder --chown=65532:65532 /data-empty /cle
# Les sauvegardes automatiques de chaque nuit, dans leur propre volume.
COPY --from=builder --chown=65532:65532 /data-empty /sauvegardes

ENV HA_DB_PATH=/data/ha.db \
    HA_KEY_PATH=/cle/ha.key \
    HA_AUTOMATIONS_PATH=/config/automations.yaml \
    HA_AREAS_PATH=/config/areas.yaml \
    HA_SCRIPTS_PATH=/config/scripts.yaml \
    HA_SCENES_PATH=/config/scenes.yaml \
    HA_ALEXA_PATH=/config/alexa.yaml \
    HA_THERMOSTATS_PATH=/config/thermostats.yaml \
    HA_BACKUP_DIR=/sauvegardes \
    HA_BIND=0.0.0.0:8123 \
    TZ=Europe/Paris

USER nonroot
WORKDIR /data
VOLUME ["/data", "/cle", "/sauvegardes"]
EXPOSE 8123

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/ha-server", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/ha-server"]
