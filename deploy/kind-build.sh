#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Build QuackGIS in the KinD cluster using in-cluster BuildKit.
# Uses registry cache for maximum layer reuse across builds.
#
# Usage:
#   ./deploy/kind-build.sh                   # build only
#   ./deploy/kind-build.sh --load            # build + load into kind nodes
#   ./deploy/kind-build.sh --deploy          # build + deploy as K8s pod
#   ./deploy/kind-build.sh --test            # build + run PostGIS regress
set -euo pipefail

cd "$(dirname "$0")/.."

BUILDER="quackgis-builder"
REGISTRY="localhost:5001"
IMAGE="${REGISTRY}/quackgis:dev"
CACHE_REF="${REGISTRY}/cache"
DOCKERFILE="container/Dockerfile.dev"
ACTION="build"

for arg in "$@"; do
    case "$arg" in
        --load) ACTION="load" ;;
        --deploy) ACTION="deploy" ;;
        --test) ACTION="test" ;;
    esac
done

echo ">> Building with in-cluster BuildKit (builder: $BUILDER)"
echo ">> Image: $IMAGE"
echo ">> Cache: $CACHE_REF"
echo

# Build with registry cache for layer reuse.
# First build: ~30 min (DuckDB compilation). Subsequent: minutes.
docker buildx build \
    --builder "$BUILDER" \
    --cache-from "type=registry,ref=${CACHE_REF}" \
    --cache-to "type=registry,ref=${CACHE_REF},mode=max" \
    -t "$IMAGE" \
    -f "$DOCKERFILE" \
    --push \
    .

echo ">> Built and pushed: $IMAGE"

case "$ACTION" in
    load)
        # Load image into all KinD nodes (for direct kubectl usage)
        echo ">> Loading image into KinD nodes..."
        kind load docker-image "$IMAGE" --name quackgis 2>/dev/null || \
            echo "   (skipped — image already in registry, nodes can pull)"
        ;;

    deploy)
        echo ">> Deploying to KinD..."
        cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: Pod
metadata:
  name: quackgis-test
  labels:
    app: quackgis
spec:
  containers:
  - name: quackgis
    image: $IMAGE
    env:
    - name: POSTGRES_PASSWORD
      value: quackgis
    ports:
    - containerPort: 5432
      hostPort: 5432
    readinessProbe:
      exec:
        command: ["sh", "-c", "pg_isready -U postgres -d postgres"]
      initialDelaySeconds: 30
      periodSeconds: 5
  restartPolicy: Never
EOF
        echo ">> Waiting for pod..."
        kubectl wait pod quackgis-test --for=condition=ready --timeout=120s 2>/dev/null || true
        echo ">> Pod status:"
        kubectl get pod quackgis-test -o wide 2>/dev/null || true
        ;;

    test)
        echo ">> Deploying + running PostGIS regress..."
        # Deploy
        $0 --deploy || true
        sleep 10

        # Port-forward
        kubectl port-forward pod/quackgis-test 55432:5432 &
        PF_PID=$!
        sleep 5
        trap "kill $PF_PID 2>/dev/null" EXIT

        # Wait for readiness
        for i in $(seq 1 60); do
            PGPASSWORD=quackgis psql -h 127.0.0.1 -p 55432 -U postgres -tAc "SELECT 1" >/dev/null 2>&1 && break
            sleep 2
        done

        # Run regress
        ./tests/run-postgis-regress.sh --report build/postgis-regress-kind.md
        ;;
esac

echo
echo ">> Done."
