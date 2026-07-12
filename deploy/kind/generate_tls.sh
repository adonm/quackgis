#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

out=${1:?usage: generate_tls.sh OUTPUT_DIRECTORY}
mkdir -p "$out"
openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 30 \
  -subj '/CN=quackgis.quackgis.svc.cluster.local' \
  -addext 'subjectAltName=DNS:quackgis,DNS:quackgis.quackgis,DNS:quackgis.quackgis.svc,DNS:quackgis.quackgis.svc.cluster.local' \
  -keyout "$out/tls.key" -out "$out/tls.crt"
cp "$out/tls.crt" "$out/ca.crt"
chmod 600 "$out/tls.key"
printf 'kind_tls_ok out=%s\n' "$out"
