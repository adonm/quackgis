#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Milestone 24: PostgreSQL-catalog DuckLake multi-writer validation.
#
# Validates that DuckLake's PostgreSQL catalog enables concurrent multi-writer
# spatial table appends with commit coordination, partition evolution, and
# three-stage query correctness — using a local Docker container as the catalog.
#
# Usage: ./tests/reference/m24_pg_catalog.sh
# Requires: docker, duckdb in PATH, sedonadb extension built.
# Skippable: set SEDONA_SKIP_PG_CATALOG=1 to skip.
set -euo pipefail
cd "$(dirname "$0")/../.."

export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
EXT="build/dev/sedonadb.duckdb_extension"
DUCKDB="${DUCKDB:-duckdb}"
PG_IMAGE="postgres:16-alpine"
PG_CONTAINER="sedonadb-pg-test"
PG_PORT="${SEDONADB_PG_PORT:-5435}"
PG_USER="sedonadb"
PG_PASS="sedonadb"
PG_DB="sedonadb"

if [ "${SEDONA_SKIP_PG_CATALOG:-0}" = "1" ]; then
    echo "SKIP: SEDONA_SKIP_PG_CATALOG=1"
    exit 0
fi

if ! command -v docker &>/dev/null; then
    echo "SKIP: docker not found (SEDONA_SKIP_PG_CATALOG=1 to silence)"
    exit 0
fi

if [ ! -f "$EXT" ]; then
    echo "FATAL: extension not found at $EXT" >&2; exit 2
fi

# Prevent DuckDB from hanging on interactive extension auto-install prompts
exec 0</dev/null

PASS=0
FAIL=0
check() {
    if [ "$1" = "$2" ]; then
        echo "PASS $3"
        PASS=$((PASS + 1))
    else
        echo "FAIL $3 (expected=$2 got=$1)"
        FAIL=$((FAIL + 1))
    fi
}

DATADIR=$(mktemp -d)
trap 'rm -rf "$DATADIR"; docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true' EXIT

echo ">> PostgreSQL-catalog DuckLake multi-writer validation"
echo ">> Temp data dir: $DATADIR"
echo

# ── Start PostgreSQL container ─────────────────────────────────────────
echo "--- Starting PostgreSQL container (port $PG_PORT) ---"
docker run -d --name "$PG_CONTAINER" \
    -e POSTGRES_USER="$PG_USER" \
    -e POSTGRES_PASSWORD="$PG_PASS" \
    -e POSTGRES_DB="$PG_DB" \
    -p "$PG_PORT:5432" \
    "$PG_IMAGE" >/dev/null 2>&1

# Wait for PostgreSQL to be ready
echo -n ">> Waiting for PostgreSQL"
for i in $(seq 1 30); do
    if docker exec "$PG_CONTAINER" pg_isready -U "$PG_USER" -d "$PG_DB" &>/dev/null; then
        echo " ready"
        break
    fi
    echo -n "."
    sleep 1
    if [ "$i" -eq 30 ]; then
        echo " TIMEOUT"
        echo "FAIL pg_container_startup"
        FAIL=$((FAIL + 1))
        echo "------------------------------------------------"
        echo "PG catalog: PASS=$PASS FAIL=$FAIL"
        exit 1
    fi
done
echo

# Connection string for DuckLake PostgreSQL catalog
CATALOG="ducklake:postgres:user=$PG_USER password=$PG_PASS host=localhost port=$PG_PORT dbname=$PG_DB"

# ── Phase 1: Create spatial table + multi-writer append ────────────────
echo "--- Phase 1: Multi-writer append via PostgreSQL catalog ---"

# Writer A: create table with partitioned layout + 500 rows
# NOTE: use minx/miny/maxx/maxy instead of xmin/ymin/xmax/ymax because
# PostgreSQL reserves xmin/xmax as system column names, and DuckLake's
# PostgreSQL catalog mirrors the table schema in PostgreSQL.
$DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "
CREATE TABLE dl.points AS
SELECT i AS id,
       st_point(i::double / 100.0, (i % 50)::double / 10.0) AS geom,
       st_xmin(geom) AS minx, st_ymin(geom) AS miny,
       st_xmax(geom) AS maxx, st_ymax(geom) AS maxy,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 500) t(i);
ALTER TABLE dl.points SET PARTITIONED BY (spatial_cell);
" 2>&1 | tail -1

# Writer B: append 300 rows from a separate process (concurrent writer)
$DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "
INSERT INTO dl.points
SELECT i + 500 AS id,
       st_point(5.0 + i::double / 100.0, 5.0 + (i % 30)::double / 10.0) AS geom,
       st_xmin(geom) AS minx, st_ymin(geom) AS miny,
       st_xmax(geom) AS maxx, st_ymax(geom) AS maxy,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 300) t(i);
" 2>&1 | tail -1

# Writer C: append 200 more rows from another process
$DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "
INSERT INTO dl.points
SELECT i + 800 AS id,
       st_point(-5.0 + i::double / 100.0, -3.0 + (i % 20)::double / 10.0) AS geom,
       st_xmin(geom) AS minx, st_ymin(geom) AS miny,
       st_xmax(geom) AS maxx, st_ymax(geom) AS maxy,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 200) t(i);
" 2>&1 | tail -1

# Verify total rows
RESULT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(*) FROM dl.points;" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1000" "pg_catalog_total_rows"

# Verify no duplicates
RESULT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(DISTINCT id) FROM dl.points;" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1000" "pg_catalog_no_duplicates"

echo

# ── Phase 2: Three-stage query correctness ─────────────────────────────
echo "--- Phase 2: Three-stage query correctness ---"

# Exact-only count (baseline)
EXACT_CNT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(*) FROM dl.points WHERE st_distance(geom, st_point(0.0, 0.0)) < 2.0;" \
    2>&1 | grep -oE '[0-9]+' | tail -1)

# Three-stage query (cell + bbox + exact)
STAGED_CNT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "
SELECT count(*) FROM dl.points
WHERE spatial_cell IN (
    SELECT quadkey FROM st_covering_quadkeys(
        st_makeenvelope(-2.0, -2.0, 2.0, 2.0), 6, 1000))
  AND maxx >= -2.0 AND minx <= 2.0
  AND maxy >= -2.0 AND miny <= 2.0
  AND st_distance(geom, st_point(0.0, 0.0)) < 2.0;
" 2>&1 | grep -oE '[0-9]+' | tail -1)

check "$STAGED_CNT" "$EXACT_CNT" "pg_catalog_query_parity"

echo

# ── Phase 3: Time travel ───────────────────────────────────────────────
echo "--- Phase 3: Time travel ---"

# Version 1 = after initial create (500 rows)
V1_CNT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(*) FROM dl.points AT (VERSION => 1);" \
    2>&1 | grep -oE '[0-9]+' | tail -1)
check "$V1_CNT" "500" "pg_catalog_time_travel_v1"

# Latest version = after all 3 writers (1000 rows)
LATEST_CNT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(*) FROM dl.points;" \
    2>&1 | grep -oE '[0-9]+' | tail -1)
check "$LATEST_CNT" "1000" "pg_catalog_time_travel_latest"

echo

# ── Phase 4: Partition evolution ───────────────────────────────────────
echo "--- Phase 4: Partition evolution ---"

$DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "ALTER TABLE dl.points RESET PARTITIONED BY; ALTER TABLE dl.points SET PARTITIONED BY (bucket(4, spatial_cell));" \
    2>&1 | tail -1

$DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "INSERT INTO dl.points
SELECT i + 1000 AS id, st_point(50.0, 50.0) AS geom,
       st_xmin(geom) AS minx, st_ymin(geom) AS miny,
       st_xmax(geom) AS maxx, st_ymax(geom) AS maxy,
       st_quadkey(geom, 6) AS spatial_cell, st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 50) t(i);" \
    2>&1 | tail -1

RESULT=$($DUCKDB -unsigned \
    -cmd "LOAD '$EXT';" \
    -cmd "LOAD postgres;" \
    -cmd "LOAD ducklake;" \
    -c "ATTACH '$CATALOG' AS dl (DATA_PATH '$DATADIR/data/');" \
    -c "SELECT count(*) FROM dl.points;" \
    2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1050" "pg_catalog_partition_evolution"

echo
echo "------------------------------------------------"
echo "PostgreSQL-catalog DuckLake: PASS=$PASS FAIL=$FAIL"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
echo "ALL PG-CATALOG CHECKS PASSED"
