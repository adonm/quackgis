#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS DuckLake storage test: create, insert, query, restart, verify
# persistence, and check three-stage query parity.
#
# Requires a running QuackGIS container with a DuckLake volume.
#
# Usage:
#   ./container/test-ducklake.sh
#
# Environment:
#   QUACKGIS_IMAGE  Image name (default: quackgis)
#   QUACKGIS_TAG    Image tag (default: dev)
#   PG_PASSWORD     Postgres password (default: quackgis)
#   PG_PORT         Host port (default: 55432)
set -euo pipefail

cd "$(dirname "$0")/.."

IMAGE="${QUACKGIS_IMAGE:-quackgis}"
TAG="${QUACKGIS_TAG:-dev}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_PORT="${PG_PORT:-55432}"
VOLUME="quackgis-ducklake-test-$$"
CONTAINER_NAME="quackgis-ducklake-test-$$"
CONTAINER_CMD="${CONTAINER_CMD:-docker}"

PASS=0
FAIL=0
FAILED_CHECKS=()

cleanup() {
    "${CONTAINER_CMD}" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    "${CONTAINER_CMD}" volume rm "$VOLUME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

psql_exec() {
    PGPASSWORD="$PG_PASSWORD" psql -h 127.0.0.1 -p "$PG_PORT" \
        -U postgres -d postgres -tA -c "$1" 2>&1
}

wait_ready() {
    echo ">> Waiting for QuackGIS to be ready..."
    for i in $(seq 1 60); do
        if "${CONTAINER_CMD}" exec "$CONTAINER_NAME" \
                pg_isready -U postgres -d postgres >/dev/null 2>&1; then
            # Give init scripts time to finish.
            sleep 5
            if psql_exec "SELECT 1;" >/dev/null 2>&1; then
                echo "   ready"
                return 0
            fi
        fi
        sleep 1
    done
    echo "   TIMEOUT"
    return 1
}

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

echo ">> Creating DuckLake volume: $VOLUME"
"${CONTAINER_CMD}" volume create "$VOLUME" >/dev/null

echo ">> Starting QuackGIS container: ${IMAGE}:${TAG}"
"${CONTAINER_CMD}" run -d \
    --name "$CONTAINER_NAME" \
    -e POSTGRES_PASSWORD="$PG_PASSWORD" \
    -e QUACKGIS_DUCKLAKE_DIR=/var/lib/quackgis \
    -p "${PG_PORT}:5432" \
    -v "${VOLUME}:/var/lib/quackgis" \
    "${IMAGE}:${TAG}" >/dev/null

wait_ready || { echo "FAIL: container not ready"; exit 1; }

export PGPASSWORD="$PG_PASSWORD"

echo
echo "── Phase 1: Create spatial DuckLake table + insert data ──────────────"

# Connect to the lake and create a test table with spatial layout.
psql_exec "SELECT quackgis.connect_lake();" || true

# Create a source table in DuckDB, then create a spatial DuckLake table.
psql_exec "SELECT * FROM duckdb.query(\$$
    CREATE TABLE _source_points AS
    SELECT i AS id,
           st_point(i::double / 10.0, (i % 100)::double / 10.0) AS geom
    FROM range(0, 500) t(i)
\$$);"

RESULT=$(psql_exec "SELECT quackgis.create_spatial_table(
    'qlake.public.test_points',
    'SELECT id, geom FROM _source_points',
    zoom := 6, bits := 12
);" 2>&1 || echo "ERROR")
echo "   create_spatial_table: $RESULT"
if echo "$RESULT" | grep -q "created"; then
    echo "PASS create spatial table"
    PASS=$((PASS + 1))
else
    echo "FAIL create spatial table ($RESULT)"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("create_spatial_table")
fi

echo
echo "── Phase 2: Query the table ──────────────────────────────────────────"

# Count rows in the DuckLake table.
RESULT=$(psql_exec "SELECT * FROM duckdb.query(\$$
    SELECT count(*) FROM qlake.public.test_points
\$$);" 2>&1 | tr -d '[:space:]')
check "row_count 500" "500" "$RESULT"

echo
echo "── Phase 3: Three-stage query parity ─────────────────────────────────"

RESULT=$(psql_exec "SELECT quackgis.spatial_query_count(
    'qlake.public.test_points',
    'POLYGON((3 1,7 1,7 5,3 5,3 1))',
    0.0, 6, 'st_intersects'
);" 2>&1 | tr -d '[:space:]')
STAGED="$RESULT"

RESULT=$(psql_exec "SELECT quackgis.exact_query_count(
    'qlake.public.test_points',
    'POLYGON((3 1,7 1,7 5,3 5,3 1))',
    'st_intersects'
);" 2>&1 | tr -d '[:space:]')
EXACT="$RESULT"

echo "   three_stage=$STAGED exact=$EXACT"
check "query_parity" "$EXACT" "$STAGED"

echo
echo "── Phase 4: Restart and verify persistence ──────────────────────────"

echo ">> Stopping container..."
"${CONTAINER_CMD}" stop "$CONTAINER_NAME" >/dev/null

echo ">> Starting container with same volume..."
"${CONTAINER_CMD}" start "$CONTAINER_NAME" >/dev/null

wait_ready || { echo "FAIL: container not ready after restart"; exit 1; }

RESULT=$(psql_exec "SELECT * FROM duckdb.query(\$$
    SELECT count(*) FROM qlake.public.test_points
\$$);" 2>&1 | tr -d '[:space:]')
check "persistence_after_restart" "500" "$RESULT"

echo
echo "── Phase 5: Pruning report ──────────────────────────────────────────"

RESULT=$(psql_exec "SELECT note FROM quackgis.pruning_report(
    'qlake.public.test_points',
    'POLYGON((3 1,7 1,7 5,3 5,3 1))',
    6, 'st_intersects'
) WHERE strategy = 'parity';" 2>&1 | tr -d '[:space:]')

if echo "$RESULT" | grep -q "PASS"; then
    echo "PASS pruning parity"
    PASS=$((PASS + 1))
else
    echo "FAIL pruning parity ($RESULT)"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("pruning_parity")
fi

echo
echo "── Summary ──────────────────────────────────────────────────────────"
echo "PASS=$PASS FAIL=$FAIL"
if [ "$FAIL" -gt 0 ]; then
    echo "Failed: ${FAILED_CHECKS[*]}"
    exit 1
fi
echo "ALL DUCKLAKE TESTS PASSED"
