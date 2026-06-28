#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Milestone 12: Multi-writer DuckLake validation.
#
# Tests that multiple DuckDB processes can append to the same spatial DuckLake
# table without data loss or duplication, and that partition evolution keeps
# queries correct across mixed layouts.
#
# Usage: ./tests/reference/m12_multiwriter.sh
# Requires: duckdb in PATH, sedonadb extension built, ducklake extension installed.
# Skippable in CI: set SEDONA_SKIP_DUCKLAKE=1 to skip.
set -euo pipefail
cd "$(dirname "$0")/../.."

export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
EXT="build/dev/sedonadb.duckdb_extension"
DUCKDB="${DUCKDB:-duckdb}"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if [ "${SEDONA_SKIP_DUCKLAKE:-0}" = "1" ]; then
    echo "SKIP: SEDONA_SKIP_DUCKLAKE=1"
    exit 0
fi

if [ ! -f "$EXT" ]; then
    echo "FATAL: extension not found at $EXT" >&2; exit 2
fi

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

echo ">> Multi-writer DuckLake validation"
echo ">> Temp dir: $TMPDIR"
echo

# ── Phase 1: Multi-process append ──────────────────────────────────────

echo "--- Phase 1: Three-process sequential append ---"

# Writer A: create table + 500 rows
$DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
CREATE TABLE dl.points AS
SELECT i AS id,
       st_point(i::double / 100.0, (i % 50)::double / 10.0) AS geom,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 500) t(i);
ALTER TABLE dl.points SET PARTITIONED BY (spatial_cell);
" 2>&1 | tail -1

# Writer B: append 300 rows from a separate process
$DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
INSERT INTO dl.points
SELECT i + 500 AS id,
       st_point(5.0 + i::double / 100.0, 5.0 + (i % 30)::double / 10.0) AS geom,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 300) t(i);
" 2>&1 | tail -1

# Writer C: append 200 more rows from another process
$DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
INSERT INTO dl.points
SELECT i + 800 AS id,
       st_point(-5.0 + i::double / 100.0, -3.0 + (i % 20)::double / 10.0) AS geom,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 200) t(i);
" 2>&1 | tail -1

# Verify
RESULT=$($DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
SELECT count(*) FROM dl.points;
" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1000" "multiwriter_total_rows"

RESULT=$($DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
SELECT count(DISTINCT id) FROM dl.points;
" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1000" "multiwriter_no_duplicates"

echo

# ── Phase 2: Partition evolution round-trip ───────────────────────────

echo "--- Phase 2: Partition evolution ---"

# Write at zoom 6 (already done above)
# Change to bucketed layout
$DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
ALTER TABLE dl.points RESET PARTITIONED BY;
ALTER TABLE dl.points SET PARTITIONED BY (bucket(4, spatial_cell));
" 2>&1 | tail -1

# Write new data under new partitioning
$DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
INSERT INTO dl.points
SELECT i + 1000 AS id,
       st_point(50.0, 50.0) AS geom,
       st_quadkey(geom, 6) AS spatial_cell,
       st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 50) t(i);
" 2>&1 | tail -1

# Verify: old + new = 1050
RESULT=$($DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
SELECT count(*) FROM dl.points;
" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "1050" "evolution_total_rows"

# Time travel: original snapshot still has 500
RESULT=$($DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
SELECT count(*) FROM dl.points AT (VERSION => 1);
" 2>&1 | grep -oE '[0-9]+' | tail -1)
check "$RESULT" "500" "evolution_time_travel_v1"

echo

# ── Phase 3: Spatial query correctness after evolution ─────────────────

echo "--- Phase 3: Query correctness ---"

# Query across mixed-layout files (zoom-6 partitioned + bucket-4 partitioned)
RESULT=$($DUCKDB -unsigned -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" -c "
ATTACH 'ducklake:$TMPDIR/mw.ducklake' AS dl (DATA_PATH '$TMPDIR/data/');
SELECT count(*) FROM dl.points WHERE st_distance(geom, st_point(0.0, 0.0)) < 2.0;
" 2>&1 | grep -oE '[0-9]+' | tail -1)
# Should find points from writer A near origin (ids 0..499, geom x=i/100)
if [ "$RESULT" -gt 0 ] 2>/dev/null; then
    echo "PASS evolution_query_nonempty ($RESULT rows)"
    PASS=$((PASS + 1))
else
    echo "FAIL evolution_query_nonempty (got=$RESULT)"
    FAIL=$((FAIL + 1))
fi

echo
echo "------------------------------------------------"
echo "Multi-writer DuckLake: PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -gt 0 ] && exit 1
echo "ALL MULTI-WRITER CHECKS PASSED"
