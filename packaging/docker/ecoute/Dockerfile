# syntax=docker/dockerfile:1
# L'écoute de la maison en Docker (étape 84, docs/voix.md) : Parakeet et
# sherpa-onnx, avec les mêmes fichiers et les mêmes empreintes que
# « installer.sh --voix » (packaging/linux/tout-compris.sh). Un fichier dont
# l'empreinte est fausse arrête la construction : rien n'est installé.
#
# Construit par compose.yaml (service « ecoute »). Pas de port publié : le
# service vit dans le réseau de ha-server, qui le joint sur 127.0.0.1:10350.

FROM busybox:1.37@sha256:bdf57e528e45e4433820e045b29b4597825a1c9e38353532d90a01445013f82e AS fichiers
ADD --checksum=sha256:cb835b6fd48a1d74809bf327b01515ec94c93428203c858885e9b09f3fe63227 \
  https://files.pythonhosted.org/packages/py3/s/sherpa-onnx-bin/sherpa_onnx_bin-1.13.8-py3-none-manylinux2014_x86_64.whl /t/bin.whl
ADD --checksum=sha256:4da90acf435373d7b2ba9cc0be806e7e274d28f63f5780880b5dea6721626e36 \
  https://files.pythonhosted.org/packages/py3/s/sherpa-onnx-core/sherpa_onnx_core-1.13.8-py3-none-manylinux2014_x86_64.whl /t/core.whl
ADD --checksum=sha256:acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247 \
  https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78/encoder.int8.onnx /ecoute/encoder.onnx
ADD --checksum=sha256:179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e \
  https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78/decoder.int8.onnx /ecoute/decoder.onnx
ADD --checksum=sha256:3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3 \
  https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78/joiner.int8.onnx /ecoute/joiner.onnx
ADD --checksum=sha256:d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d \
  https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78/tokens.txt /ecoute/tokens.txt
RUN cd /t \
  && unzip -q bin.whl sherpa_onnx_bin-1.13.8.data/data/bin/sherpa-onnx-offline-websocket-server \
  && unzip -q core.whl sherpa_onnx/lib/libonnxruntime.so \
  && mv sherpa_onnx_bin-1.13.8.data/data/bin/sherpa-onnx-offline-websocket-server /ecoute/ \
  && mv sherpa_onnx/lib/libonnxruntime.so /ecoute/ \
  && chmod 0755 /ecoute/sherpa-onnx-offline-websocket-server \
  && chmod 0644 /ecoute/*.onnx /ecoute/tokens.txt /ecoute/libonnxruntime.so

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
COPY --from=fichiers /ecoute /ecoute
ENV LD_LIBRARY_PATH=/ecoute
# Aucun privilège : un compte sans nom, sans droit d'écrire.
USER 65532:65532
ENTRYPOINT ["/ecoute/sherpa-onnx-offline-websocket-server", \
  "--port=10350", \
  "--encoder=/ecoute/encoder.onnx", \
  "--decoder=/ecoute/decoder.onnx", \
  "--joiner=/ecoute/joiner.onnx", \
  "--tokens=/ecoute/tokens.txt", \
  "--model-type=nemo_transducer", \
  "--num-threads=4", \
  "--num-work-threads=1", \
  "--max-batch-size=1", \
  "--max-utterance-length=20", \
  "--log-file=/dev/null"]
