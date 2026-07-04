#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Set up in-cluster BuildKit in KinD with persistent cache.
#
# Creates:
#   1. A KinD cluster with a local registry (localhost:5001)
#   2. A BuildKit builder via `docker buildx create --driver kubernetes`
#      with a 20Gi PVC for cache persistence
#   3. Registry cache configured for maximum layer reuse
#
# Usage:
#   ./deploy/setup-buildkit.sh              # create everything
#   ./deploy/setup-buildkit.sh --teardown   # destroy everything
set -euo pipefail

cd "$(dirname "$0")/.."

CLUSTER_NAME="quackgis"
REGISTRY_NAME="kind-registry"
REGISTRY_PORT="5001"
BUILDER_NAME="quackgis-builder"
NAMESPACE="buildkit"

TEARDOWN=false
for arg in "$@"; do
    case "$arg" in
        --teardown) TEARDOWN=true ;;
    esac
done

if $TEARDOWN; then
    echo ">> Tearing down BuildKit + KinD..."
    docker buildx rm "$BUILDER_NAME" 2>/dev/null || true
    kubectl delete namespace "$NAMESPACE" 2>/dev/null || true
    docker stop "$REGISTRY_NAME" 2>/dev/null || true
    docker rm "$REGISTRY_NAME" 2>/dev/null || true
    kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
    echo ">> Done."
    exit 0
fi

# ── 1. Create KinD cluster with registry ────────────────────────────────────

if ! kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
    echo ">> Creating KinD cluster: $CLUSTER_NAME"

    # Create cluster FIRST (without registry port conflicts).
    cat <<EOF | kind create cluster --name "$CLUSTER_NAME" --config=-
apiVersion: kind.x-k8s.io/v1alpha4
kind: Cluster
containerdConfigPatches:
  - |-
    [plugins."io.containerd.grpc.v1.cri".registry.mirrors."localhost:${REGISTRY_PORT}"]
      endpoint = ["http://${REGISTRY_NAME}:5000"]
nodes:
  - role: control-plane
    extraPortMappings:
      - containerPort: 80
        hostPort: 8080
        protocol: TCP
EOF

    # Start local registry container connected to the kind network.
    docker run -d --restart=always -p "${REGISTRY_PORT}:5000" \
        --net kind \
        --name "$REGISTRY_NAME" registry:2

    # Document the local registry
    cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: ConfigMap
metadata:
  name: local-registry-hosting
  namespace: kube-public
data:
  localRegistryHosting.v1: |
    host: "localhost:${REGISTRY_PORT}"
    help: "https://kind.sigs.k8s.io/docs/user/local-registry/"
EOF
else
    echo ">> KinD cluster $CLUSTER_NAME already exists"
fi

# ── 2. Create BuildKit builder in the cluster ───────────────────────────────

echo ">> Creating BuildKit builder: $BUILDER_NAME"

# Remove existing builder if present
docker buildx rm "$BUILDER_NAME" 2>/dev/null || true

# Create namespace for BuildKit
kubectl create namespace "$NAMESPACE" 2>/dev/null || true

# Create BuildKit deployment in Kubernetes with PVC for cache persistence.
# The PVC survives pod restarts so build cache is not lost.
docker buildx create \
    --name "$BUILDER_NAME" \
    --driver kubernetes \
    --driver-opt namespace="$NAMESPACE" \
    --driver-opt image=moby/buildkit:v0.22.0 \
    --driver-opt "replicas=1" \
    --driver-opt "rootless=false" \
    --driver-opt "requests.cpu=2" \
    --driver-opt "requests.memory=4Gi" \
    --driver-opt "limits.cpu=4" \
    --driver-opt "limits.memory=8Gi" \
    --driver-opt "persistent-volume-claim.requests.storage=20Gi" \
    --bootstrap

# Verify builder
echo ">> BuildKit builder ready:"
docker buildx inspect "$BUILDER_NAME" --bootstrap | head -10

# ── 3. Verify registry ──────────────────────────────────────────────────────

echo
echo ">> Local registry: localhost:${REGISTRY_PORT}"
echo ">> BuildKit builder: $BUILDER_NAME (in namespace: $NAMESPACE)"
echo ">> Cache PVC: 20Gi (persists across pod restarts)"
echo
echo "Usage:"
echo "  docker buildx build --builder $BUILDER_NAME \\"
echo "    --cache-to type=registry,ref=localhost:${REGISTRY_PORT}/cache,mode=max \\"
echo "    --cache-from type=registry,ref=localhost:${REGISTRY_PORT}/cache \\"
echo "    -t localhost:${REGISTRY_PORT}/quackgis:dev \\"
echo "    -f container/Dockerfile.dev ."
echo "  kubectl run quackgis-test --image=localhost:${REGISTRY_PORT}/quackgis:dev ..."
