# syntax=docker/dockerfile:1
# Le cerveau de la maison en Docker (docs/cerveau.md) : Claude Code, qui
# répond aux phrases qu'Assist ne comprend pas, avec l'abonnement Claude de
# la maison (pas de clé d'API payante).
#
# Claude Code est téléchargé depuis npm à la construction, à une version
# fixée, et vérifié par son empreinte : une empreinte fausse arrête la
# construction. Il n'est pas dans le dépôt (licence d'Anthropic).
#
# Construit par compose.yaml (service « cerveau »). Pas de port publié :
# seul ha-server lui parle, avec la clé partagée.

FROM busybox:1.37@sha256:bdf57e528e45e4433820e045b29b4597825a1c9e38353532d90a01445013f82e AS fichiers
ADD --checksum=sha256:25d2e2cae6d3d1d5ceeaf0da02e83c45c16455e45efa1ab305395dc05227ad0d \
  https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.220.tgz /t/claude.tgz
RUN mkdir /cerveau \
  && tar -xzf /t/claude.tgz -C /t package/claude \
  && mv /t/package/claude /cerveau/claude \
  && chmod 0755 /cerveau/claude

FROM node:22-bookworm-slim@sha256:5ec23b7c622b00547753bc6ffd78085dd868a68aeb41c8e0a1418f033e13d96d
COPY --from=fichiers /cerveau/claude /cerveau/claude
COPY serveur.mjs /cerveau/serveur.mjs
# Aucun privilège : un compte sans nom, sans droit d'écrire (sauf /tmp).
USER 65532:65532
ENV NODE_ENV=production
HEALTHCHECK --interval=60s --timeout=5s --start-period=10s \
  CMD ["node", "-e", "fetch('http://127.0.0.1:8790/sante').then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"]
ENTRYPOINT ["node", "/cerveau/serveur.mjs"]
