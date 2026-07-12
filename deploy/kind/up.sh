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

cluster_exists=false
if kind get clusters | grep -qx quackgis; then
  cluster_exists=true
  kind export kubeconfig --name quackgis --kubeconfig "$kubeconfig"
  if ! kubectl --request-timeout=5s get --raw=/readyz >/dev/null 2>&1; then
    printf 'kind_cluster_stale provider=%s cluster=quackgis action=recreate\n' \
      "$KIND_EXPERIMENTAL_PROVIDER" >&2
    kind delete cluster --name quackgis
    cluster_exists=false
  fi
fi
if [ "$cluster_exists" = false ]; then
  kind create cluster \
    --config "$root/deploy/kind/cluster.yaml" \
    --kubeconfig "$kubeconfig" \
    --wait 5m
fi
load_local_image() {
  source_image=$1
  archive_name=$2
  archive="$work/$archive_name.tar"
  rm -f "$archive"
  if [ "$engine" = podman ]; then
    "$engine" image save --format docker-archive --output "$archive" "$source_image"
  else
    "$engine" image save --output "$archive" "$source_image"
  fi
  kind load image-archive "$archive" --name quackgis
  rm -f "$archive"
}

if [ -n "${QUACKGIS_RUNTIME_LOAD_IMAGE:-}" ]; then
  load_local_image "$QUACKGIS_RUNTIME_LOAD_IMAGE" runtime-image
fi
if [ -n "${QUACKGIS_CLIENT_LOAD_IMAGE:-}" ]; then
  load_local_image "$QUACKGIS_CLIENT_LOAD_IMAGE" client-image
fi

node_digest_reference() {
  source_image=$1
  first_node=$(kind get nodes --name quackgis | head -n 1)
  digest=$(
    "$engine" exec "$first_node" ctr --namespace k8s.io images list |
      awk -v image="$source_image" '$1 == image { print $3; exit }'
  )
  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????) ;;
    *) printf 'loaded image has no containerd manifest digest: %s\n' "$source_image" >&2; exit 2 ;;
  esac
  repository=$(python3 -c 'import sys
value = sys.argv[1].split("@", 1)[0]
slash = value.rfind("/")
colon = value.rfind(":")
print(value[:colon] if colon > slash else value)' "$source_image")
  reference="$repository@$digest"
  for node in $(kind get nodes --name quackgis); do
    if ! "$engine" exec "$node" ctr --namespace k8s.io images tag "$source_image" "$reference" >/dev/null 2>&1; then
      "$engine" exec "$node" crictl inspecti "$reference" >/dev/null
    fi
  done
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
