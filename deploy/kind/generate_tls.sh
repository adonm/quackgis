#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

out=${1:?usage: generate_tls.sh OUTPUT_DIRECTORY}
mkdir -p "$out"
umask 077
openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 30 \
  -subj '/CN=QuackGIS Kind development CA' \
  -keyout "$out/ca.key" -out "$out/ca.crt" 2>/dev/null
openssl req -new -newkey rsa:2048 -sha256 -nodes \
  -subj '/CN=quackgis.quackgis.svc.cluster.local' \
  -addext 'subjectAltName=DNS:quackgis,DNS:quackgis.quackgis,DNS:quackgis.quackgis.svc,DNS:quackgis.quackgis.svc.cluster.local' \
  -addext 'extendedKeyUsage=serverAuth' \
  -keyout "$out/tls.key" -out "$out/server.csr" 2>/dev/null
openssl x509 -req -sha256 -days 30 -copy_extensions copy \
  -in "$out/server.csr" -CA "$out/ca.crt" -CAkey "$out/ca.key" -CAcreateserial \
  -out "$out/tls.crt" 2>/dev/null
openssl req -new -newkey rsa:2048 -sha256 -nodes \
  -subj '/CN=quackgis-kind-clients' \
  -addext 'extendedKeyUsage=clientAuth' \
  -keyout "$out/client.key" -out "$out/client.csr" 2>/dev/null
openssl x509 -req -sha256 -days 30 -copy_extensions copy \
  -in "$out/client.csr" -CA "$out/ca.crt" -CAkey "$out/ca.key" -CAcreateserial \
  -out "$out/client.crt" 2>/dev/null
rm -f "$out/server.csr" "$out/client.csr" "$out/ca.srl"
chmod 600 "$out/ca.key" "$out/tls.key" "$out/client.key"
printf 'kind_tls_ok out=%s\n' "$out"
