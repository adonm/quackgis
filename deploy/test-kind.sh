#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS Kubernetes smoke test (Milestone 5).
#
# Creates a KinD cluster, loads the image, applies manifests, runs a spatial
# query through psql, restarts the pod, and verifies persistence.
#
# Requires: docker, kind, kubectl, psql.
#
# Usage:
#   ./deploy/test-kind.sh
#
# Environment:
#   QUACKGIS_IMAGE  Image name (default: quackgis)
#   QUACKGIS_TAG    Image tag (default: dev)
#   SKIP_IMAGE_BUILD  If set, skip building the image
set -uo pipefail

cd "$(dirname "$0")/.."

IMAGE="${QUACKGIS_IMAGE:-quackgis}"
TAG="${QUACKGIS_TAG:-dev}"
CLUSTER_NAME="quackgis-kind-test"
NAMESPACE="quackgis"

PASS=0
FAIL=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo "PASS $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL $label (expected='$expected' got='$actual')"
        FAIL=$((FAIL + 1))
    fi
}

# ── Prerequisites ────────────────────────────────────────────────────────────

for cmd in docker kind kubectl psql; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "SKIP: $cmd not found"
        exit 0
    fi
done

# ── Build image (optional) ───────────────────────────────────────────────────

if [ -z "${SKIP_IMAGE_BUILD:-}" ]; then
    echo "── Build image ──────────────────────────────────────────────"
    ./container/build.sh -t "$TAG" || { echo "BUILD FAILED"; exit 1; }
fi

# ── Create KinD cluster ──────────────────────────────────────────────────────

echo "── Create KinD cluster: $CLUSTER_NAME ────────────────────────"

# Delete existing cluster if present.
kind delete cluster --name "$CLUSTER_NAME" >/dev/null 2>&1 || true

kind create cluster --name "$CLUSTER_NAME" >/dev/null 2>&1 || {
    echo "FAIL: could not create KinD cluster"
    exit 1
}

# Load the image into KinD.
echo ">> Loading ${IMAGE}:${TAG} into KinD..."
kind load docker-image "${IMAGE}:${TAG}" --name "$CLUSTER_NAME" || {
    echo "FAIL: could not load image into KinD"
    kind delete cluster --name "$CLUSTER_NAME" >/dev/null 2>&1
    exit 1
}

# ── Apply manifests ──────────────────────────────────────────────────────────

echo "── Apply K8s manifests ──────────────────────────────────────"
kubectl apply -f deploy/k8s/quackgis.yaml

# ── Wait for readiness ───────────────────────────────────────────────────────

echo ">> Waiting for pod to be ready..."
kubectl wait pod -n "$NAMESPACE" -l app=quackgis \
    --for=condition=ready --timeout=120s || {
    echo "FAIL: pod did not become ready"
    kubectl describe pod -n "$NAMESPACE" -l app=quackgis
    kubectl logs -n "$NAMESPACE" -l app=quackgis --tail=50
    kind delete cluster --name "$CLUSTER_NAME"
    exit 1
}
echo "   ready"

# ── Port-forward ─────────────────────────────────────────────────────────────

echo ">> Port-forwarding..."
kubectl port-forward -n "$NAMESPACE" svc/quackgis 55432:5432 &
PF_PID=$!
sleep 3

cleanup() {
    kill $PF_PID 2>/dev/null || true
    kind delete cluster --name "$CLUSTER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

export PGPASSWORD=quackgis
PSQL="psql -h 127.0.0.1 -p 55432 -U postgres -d postgres -tA"

echo
echo "── Phase 1: Spatial smoke ────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_astext(st_geomfromtext('POINT(1 2)'));" 2>&1)
check "st_astext POINT" "POINT(1 2)" "$RESULT"

RESULT=$($PSQL -c "SELECT postgis_version();" 2>&1 | grep -c QUACKGIS || true)
check "postgis_version" "1" "$RESULT"

RESULT=$($PSQL -c "SELECT st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));" 2>&1)
check "st_area" "16" "$RESULT"

echo
echo "── Phase 2: DuckLake persistence ────────────────────────────"

# Create a test table via DuckDB/DuckLake.
$PSQL -c "SELECT quackgis.connect_lake();" 2>&1 || true

$PSQL -c "SELECT * FROM duckdb.query(\$$
    CREATE TABLE _k8s_source AS
    SELECT i AS id, st_point(i::double, i::double) AS geom
    FROM range(0, 100) t(i)
\$$);" 2>&1 || true

RESULT=$($PSQL -c "SELECT quackgis.create_spatial_table(
    'qlake.public.k8s_points',
    'SELECT id, geom FROM _k8s_source',
    zoom := 6, bits := 12
);" 2>&1)
echo "   create: $RESULT"

RESULT=$($PSQL -c "SELECT * FROM duckdb.query(\$$
    SELECT count(*) FROM qlake.public.k8s_points
\$$);" 2>&1 | tr -d '[:space:]')
check "ducklake row_count" "100" "$RESULT"

echo
echo "── Phase 3: Restart pod and verify persistence ──────────────"

echo ">> Deleting pod to trigger restart..."
kubectl delete pod -n "$NAMESPACE" -l app=quackgis --grace-period=5 >/dev/null 2>&1

echo ">> Waiting for new pod to be ready..."
kubectl wait pod -n "$NAMESPACE" -l app=quackgis \
    --for=condition=ready --timeout=120s || {
    echo "FAIL: pod did not become ready after restart"
    exit 1
}

# Re-establish port-forward (the old one died with the pod).
sleep 5
kubectl port-forward -n "$NAMESPACE" svc/quackgis 55432:5432 &
PF_PID2=$!
sleep 3

RESULT=$($PSQL -c "SELECT * FROM duckdb.query(\$$
    SELECT count(*) FROM qlake.public.k8s_points
\$$);" 2>&1 | tr -d '[:space:]')
check "persistence after restart" "100" "$RESULT"

kill $PF_PID2 2>/dev/null || true

echo
echo "── Summary ──────────────────────────────────────────────────"
echo "PASS=$PASS FAIL=$FAIL"
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
echo "ALL K8S SMOKE TESTS PASSED"
