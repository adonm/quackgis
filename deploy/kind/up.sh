#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
tls="$work/tls"
edge="$work/edge"
rest="$work/rest"
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
command -v kind >/dev/null
command -v kubectl >/dev/null
tls_valid=true
for file in ca.crt ca.key tls.crt tls.key client.crt client.key; do
  if [ ! -f "$tls/$file" ]; then tls_valid=false; fi
done
if [ "$tls_valid" = true ]; then
  openssl verify -purpose sslserver -verify_hostname quackgis.quackgis.svc.cluster.local \
    -CAfile "$tls/ca.crt" "$tls/tls.crt" >/dev/null 2>&1 || tls_valid=false
  openssl verify -purpose sslclient -CAfile "$tls/ca.crt" "$tls/client.crt" >/dev/null 2>&1 || tls_valid=false
  openssl x509 -checkend 86400 -noout -in "$tls/ca.crt" >/dev/null 2>&1 || tls_valid=false
  openssl x509 -checkend 86400 -noout -in "$tls/tls.crt" >/dev/null 2>&1 || tls_valid=false
  openssl x509 -checkend 86400 -noout -in "$tls/client.crt" >/dev/null 2>&1 || tls_valid=false
  server_cert=$(openssl x509 -in "$tls/tls.crt" -pubkey -noout | sha256sum | cut -d' ' -f1)
  server_key=$(openssl pkey -in "$tls/tls.key" -pubout | sha256sum | cut -d' ' -f1)
  client_cert=$(openssl x509 -in "$tls/client.crt" -pubkey -noout | sha256sum | cut -d' ' -f1)
  client_key=$(openssl pkey -in "$tls/client.key" -pubout | sha256sum | cut -d' ' -f1)
  if [ "$server_cert" != "$server_key" ] || [ "$client_cert" != "$client_key" ]; then tls_valid=false; fi
fi
if [ "$tls_valid" = false ]; then
  rm -rf "$tls"
  "$root/deploy/kind/generate_tls.sh" "$tls"
fi

keygen=${QUACKGIS_KEYGEN:-$root/target/release/quackgis-keygen}
if [ ! -x "$keygen" ]; then
  printf 'quackgis-keygen is missing; build the runtime context or set QUACKGIS_KEYGEN\n' >&2
  exit 2
fi
mkdir -p "$edge" "$rest"
for name in bootstrap worker credential client-transport rest-credential; do
  if [ ! -f "$edge/$name.key" ]; then
    "$keygen" --out "$edge/$name.key" >/dev/null
  fi
done
jwt_valid=false
if [ -f "$rest/jwt-secret" ] && JWT_SECRET_FILE="$rest/jwt-secret" python3 - <<'PY'
import os
value = open(os.environ["JWT_SECRET_FILE"], "rb").read()
raise SystemExit(0 if 32 <= len(value) <= 4096 and not any(byte in b" \t\n\r\v\f" for byte in value) else 1)
PY
then
  jwt_valid=true
fi
if [ "$jwt_valid" = false ]; then
  openssl rand -hex 48 >"$rest/jwt-secret"
  chmod 600 "$rest/jwt-secret"
fi
bootstrap_public_key=$("$keygen" --public-from "$edge/bootstrap.key")
worker_public_key=$("$keygen" --public-from "$edge/worker.key")
credential_public_key=$("$keygen" --public-from "$edge/credential.key")
rest_credential_public_key=$("$keygen" --public-from "$edge/rest-credential.key")

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
  --edge-dir "$edge" \
  --bootstrap-public-key "$bootstrap_public_key" \
  --worker-public-key "$worker_public_key" \
  --credential-public-key "$credential_public_key" \
  --rest-credential-public-key "$rest_credential_public_key" \
  --jwt-secret-file "$rest/jwt-secret" \
  --out-dir "$rendered"

current_service=$(kubectl -n quackgis get statefulset quackgis -o jsonpath='{.spec.serviceName}' 2>/dev/null || true)
if [ -n "$current_service" ] && [ "$current_service" != quackgis-edge-internal ]; then
  printf 'kind_statefulset_replace old_service=%s new_service=quackgis-edge-internal\n' "$current_service"
  kubectl -n quackgis delete statefulset quackgis --cascade=foreground --wait=true
fi
kubectl apply -f "$rendered/core.yaml"
desired_hash=$(kubectl -n quackgis get statefulset quackgis -o jsonpath='{.spec.template.metadata.annotations.quackgis\.dev/package-config-sha256}')
pod_hash=$(kubectl -n quackgis get pod quackgis-0 -o jsonpath='{.metadata.annotations.quackgis\.dev/package-config-sha256}' 2>/dev/null || true)
if [ -n "$pod_hash" ] && [ "$pod_hash" != "$desired_hash" ]; then
  printf 'kind_pod_replace old_config=%s new_config=%s\n' "$pod_hash" "$desired_hash"
  kubectl -n quackgis delete pod quackgis-0 --wait=false
fi
kubectl -n quackgis rollout status statefulset/quackgis --timeout=5m
kubectl -n quackgis delete job quackgis-rest-seed --ignore-not-found --wait=true >/dev/null
kubectl apply -f "$rendered/rest-seed.yaml"
if ! kubectl -n quackgis wait --for=condition=complete job/quackgis-rest-seed --timeout=2m; then
  kubectl -n quackgis logs job/quackgis-rest-seed --all-containers=true || true
  exit 1
fi
kubectl -n quackgis logs job/quackgis-rest-seed --all-containers=true
kubectl apply -f "$rendered/rest.yaml"
kubectl -n quackgis rollout status deployment/quackgis-rest --timeout=5m
ready_rest=$(kubectl -n quackgis get deployment quackgis-rest -o jsonpath='{.status.readyReplicas}')
if [ "$ready_rest" != 2 ]; then
  printf 'expected two ready REST replicas, got %s\n' "${ready_rest:-0}" >&2
  exit 1
fi
printf 'kind_up_ok provider=%s context=kind-quackgis kubeconfig=%s clients=%s rest_replicas=2\n' \
  "$KIND_EXPERIMENTAL_PROVIDER" "$kubeconfig" "$rendered/clients.yaml"
