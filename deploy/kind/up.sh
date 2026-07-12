#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
tls="$work/tls"
rendered="$work/rendered"
kubeconfig=${KUBECONFIG:-$work/kubeconfig}

engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
  engine=$(python3 "$root/scripts/project_doctor.py" --container-engine)
fi
case "$engine" in
  podman|docker) ;;
  *) printf 'unsupported Kind container engine: %s\n' "$engine" >&2; exit 2 ;;
esac
: "${KIND_EXPERIMENTAL_PROVIDER:=$engine}"
export KIND_EXPERIMENTAL_PROVIDER KUBECONFIG="$kubeconfig"

QUACKGIS_RUNTIME_IMAGE=${QUACKGIS_RUNTIME_IMAGE:-}
QUACKGIS_CLIENT_IMAGE=${QUACKGIS_CLIENT_IMAGE:-}
if [ -z "$QUACKGIS_RUNTIME_IMAGE" ] && [ -z "${QUACKGIS_RUNTIME_LOAD_IMAGE:-}" ]; then
  printf 'set digest-pinned QUACKGIS_RUNTIME_IMAGE or QUACKGIS_RUNTIME_LOAD_IMAGE\n' >&2
  exit 2
fi
if [ -z "$QUACKGIS_CLIENT_IMAGE" ] && [ -z "${QUACKGIS_CLIENT_LOAD_IMAGE:-}" ]; then
  printf 'set digest-pinned QUACKGIS_CLIENT_IMAGE or QUACKGIS_CLIENT_LOAD_IMAGE\n' >&2
  exit 2
fi
: "${QUACKGIS_AUTH_PASSWORD_FILE:?set QUACKGIS_AUTH_PASSWORD_FILE}"

command -v kind >/dev/null
command -v kubectl >/dev/null
if [ ! -f "$tls/tls.crt" ]; then
  "$root/deploy/kind/generate_tls.sh" "$tls"
fi

if ! kind get clusters | grep -qx quackgis; then
  kind create cluster \
    --config "$root/deploy/kind/cluster.yaml" \
    --kubeconfig "$kubeconfig" \
    --wait 5m
else
  kind export kubeconfig --name quackgis --kubeconfig "$kubeconfig"
fi
if [ -n "${QUACKGIS_RUNTIME_LOAD_IMAGE:-}" ]; then
  kind load docker-image "$QUACKGIS_RUNTIME_LOAD_IMAGE" --name quackgis
fi
if [ -n "${QUACKGIS_CLIENT_LOAD_IMAGE:-}" ]; then
  kind load docker-image "$QUACKGIS_CLIENT_LOAD_IMAGE" --name quackgis
fi

node_digest_reference() {
  source_image=$1
  reference=$(
    "$engine" exec quackgis-control-plane crictl inspecti "$source_image" |
      python3 -c 'import json, sys
status = json.load(sys.stdin).get("status", {})
digests = status.get("repoDigests", [])
if not digests:
    raise SystemExit("loaded image has no CRI repository digest")
print(digests[0])'
  )
  printf '%s\n' "$reference"
}

if [ -z "$QUACKGIS_RUNTIME_IMAGE" ]; then
  QUACKGIS_RUNTIME_IMAGE=$(node_digest_reference "$QUACKGIS_RUNTIME_LOAD_IMAGE")
fi
if [ -z "$QUACKGIS_CLIENT_IMAGE" ]; then
  QUACKGIS_CLIENT_IMAGE=$(node_digest_reference "$QUACKGIS_CLIENT_LOAD_IMAGE")
fi
python3 "$root/deploy/kind/render.py" \
  --runtime-image "$QUACKGIS_RUNTIME_IMAGE" \
  --client-image "$QUACKGIS_CLIENT_IMAGE" \
  --tls-dir "$tls" \
  --password-file "$QUACKGIS_AUTH_PASSWORD_FILE" \
  --out-dir "$rendered"

kubectl apply -f "$rendered/core.yaml"
kubectl -n quackgis rollout status statefulset/quackgis --timeout=5m
printf 'kind_up_ok provider=%s context=kind-quackgis kubeconfig=%s clients=%s\n' \
  "$KIND_EXPERIMENTAL_PROVIDER" "$kubeconfig" "$rendered/clients.yaml"
