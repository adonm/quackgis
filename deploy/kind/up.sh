#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

: "${QUACKGIS_RUNTIME_IMAGE:?set digest-pinned QUACKGIS_RUNTIME_IMAGE}"
: "${QUACKGIS_CLIENT_IMAGE:?set digest-pinned QUACKGIS_CLIENT_IMAGE}"
: "${QUACKGIS_AUTH_PASSWORD_FILE:?set QUACKGIS_AUTH_PASSWORD_FILE}"

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
tls="$work/tls"
rendered="$work/rendered"

command -v kind >/dev/null
command -v kubectl >/dev/null
if [ ! -f "$tls/tls.crt" ]; then
  "$root/deploy/kind/generate_tls.sh" "$tls"
fi
python3 "$root/deploy/kind/render.py" \
  --runtime-image "$QUACKGIS_RUNTIME_IMAGE" \
  --client-image "$QUACKGIS_CLIENT_IMAGE" \
  --tls-dir "$tls" \
  --password-file "$QUACKGIS_AUTH_PASSWORD_FILE" \
  --out-dir "$rendered"

if ! kind get clusters | grep -qx quackgis; then
  kind create cluster --config "$root/deploy/kind/cluster.yaml"
fi
kubectl apply -f "$rendered/core.yaml"
kubectl -n quackgis rollout status statefulset/quackgis --timeout=5m
printf 'kind_up_ok context=kind-quackgis clients=%s\n' "$rendered/clients.yaml"
