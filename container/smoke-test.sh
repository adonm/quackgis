#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Black-box smoke test for the QuackGIS facade container.
#
# Starts a fresh container, waits for readiness, runs spatial queries through
# psql, and reports PASS/FAIL. Requires docker (or podman) and psql.
#
# Usage:
#   ./container/smoke-test.sh
#
# Environment:
#   QUACKGIS_IMAGE  Image name (default: quackgis)
#   QUACKGIS_TAG    Image tag (default: dev)
#   PG_PASSWORD     Postgres password (default: quackgis)
#   PG_PORT         Host port to map (default: 55432)
set -euo pipefail

cd "$(dirname "$0")/.."

IMAGE="${QUACKGIS_IMAGE:-quackgis}"
TAG="${QUACKGIS_TAG:-dev}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_PORT="${PG_PORT:-55432}"
CONTAINER_NAME="quackgis-smoke-$$"
CONTAINER_CMD="${CONTAINER_CMD:-docker}"

PASS=0
FAIL=0
FAILED_CHECKS=()

cleanup() {
    "${CONTAINER_CMD}" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo ">> Starting QuackGIS container: ${IMAGE}:${TAG}"
"${CONTAINER_CMD}" run -d \
    --name "$CONTAINER_NAME" \
    -e POSTGRES_PASSWORD="$PG_PASSWORD" \
    -p "${PG_PORT}:5432" \
    "${IMAGE}:${TAG}" >/dev/null

export PGPASSWORD="$PG_PASSWORD"
PSQL="psql -h 127.0.0.1 -p $PG_PORT -U postgres -d postgres -tA"

echo ">> Waiting for Postgres to be ready..."
for i in $(seq 1 30); do
    if "${CONTAINER_CMD}" exec "$CONTAINER_NAME" \
            pg_isready -U postgres -d postgres >/dev/null 2>&1; then
        echo "   ready"
        break
    fi
    sleep 1
    if [ "$i" -eq 30 ]; then
        echo "   TIMEOUT"
        echo "FAIL: container did not become ready"
        exit 1
    fi
done

# Give init scripts a moment to complete
sleep 5

check() {
    local label="$1"
    local expected="$2"
    local actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo "PASS $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL $label (expected='$expected' got='$actual')"
        FAIL=$((FAIL + 1))
        FAILED_CHECKS+=("$label")
    fi
}

echo
echo "── Diagnostics ──────────────────────────────────────────────"
$PSQL -c "SELECT * FROM quackgis.diagnostics;" 2>&1 || true
echo

echo "── Smoke checks ─────────────────────────────────────────────"

# 1. ST_AsText via wrapper
RESULT=$($PSQL -c "SELECT quackgis.smoke_check();" 2>&1 || echo "ERROR")
check "smoke_check POINT(0 0)" "POINT(0 0)" "$RESULT"

# 2. ST_Area of a unit square
RESULT=$($PSQL -c "SELECT st_area(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'));" 2>&1 || echo "ERROR")
check "st_area unit square" "1" "$RESULT"

# 3. ST_Distance between two points
RESULT=$($PSQL -c "SELECT st_distance(st_point(0,0), st_point(3,4));" 2>&1 || echo "ERROR")
check "st_distance 3-4-5" "5" "$RESULT"

# 4. ST_Intersects overlapping polygons
RESULT=$($PSQL -c "SELECT st_intersects(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'));" 2>&1 || echo "ERROR")
check "st_intersects overlap" "t" "$RESULT"

# 5. DuckDB version is visible
RESULT=$($PSQL -c "SELECT (duckdb.query(\$$ SELECT duckdb_version() \$$) -> 0)::text;" 2>&1 || echo "ERROR")
if [ "$RESULT" != "ERROR" ] && [ -n "$RESULT" ]; then
    echo "PASS duckdb_version visible ($RESULT)"
    PASS=$((PASS + 1))
else
    echo "FAIL duckdb_version visible"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("duckdb_version")
fi

# 6. sedonadb extension is loaded in DuckDB
RESULT=$($PSQL -c "SELECT count(*) FROM duckdb.query(\$$ SELECT 1 FROM duckdb_extensions() WHERE loaded AND extension_name = 'sedonadb' \$$);" 2>&1 || echo "0")
check "sedonadb loaded" "1" "$RESULT"

echo
echo "── Summary ──────────────────────────────────────────────────"
echo "PASS=$PASS FAIL=$FAIL"
if [ "$FAIL" -gt 0 ]; then
    echo "Failed: ${FAILED_CHECKS[*]}"
    exit 1
fi
echo "ALL SMOKE CHECKS PASSED"
