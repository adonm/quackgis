#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Scale validation harness: generate deterministic spatial data at configurable
# tiers, create three DuckLake layouts, run representative queries, and report
# layout metrics + exact-result parity.
#
# This is the reproducible evidence that the DuckLake spatial lakehouse pattern
# is correct at scale and that pruning is effective. It is NOT a microbenchmark
# — timing is informational; exact-result parity is the oracle.
#
# Usage: ./benchmarks/scale_harness.sh [TIER]
#   TIER: smoke (1k points), local (10k), heavy (100k). Default: smoke.
set -euo pipefail
cd "$(dirname "$0")/.."

# Locate runtime libs.
if [ -d /var/home/linuxbrew/.linuxbrew/lib ]; then
    export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

TIER="${1:-smoke}"
EXT="build/dev/sedonadb.duckdb_extension"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

case "$TIER" in
    smoke)  N=1000;  QZOOM=4 ;;
    local)  N=10000; QZOOM=5 ;;
    heavy)  N=100000; QZOOM=6 ;;
    *) echo "Unknown tier: $TIER (use smoke|local|heavy)"; exit 1 ;;
esac

echo ">> Scale harness: tier=$TIER  points=$N  query_zoom=$QZOOM"
echo ">> Temp dir: $TMPDIR"
echo

# ---- Generate data and create three layouts ----
duckdb -unsigned \
  -cmd "LOAD '$EXT';" \
  -cmd "LOAD ducklake;" \
  -c "
-- Deterministic source data: 80% uniform, 20% clustered near origin.
CREATE TABLE source AS
SELECT
    i AS id,
    CASE WHEN i % 5 = 0
         THEN st_point((i % 100)::double / 10.0 - 5.0,
                       ((i * 3) % 100)::double / 10.0 - 5.0)
         ELSE st_point((i % 1000)::double / 10.0 - 50.0,
                       ((i * 7) % 1000)::double / 10.0 - 50.0)
    END AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, $QZOOM) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM range(0, $N) t(i);

-- Layout 1: flat (no layout columns)
ATTACH 'ducklake:$TMPDIR/l1.ducklake' AS dl1 (DATA_PATH '$TMPDIR/l1/');
CREATE TABLE dl1.flat AS SELECT id, geom FROM source;

-- Layout 2: bbox + Hilbert-sorted (zone-map pruning)
ATTACH 'ducklake:$TMPDIR/l2.ducklake' AS dl2 (DATA_PATH '$TMPDIR/l2/');
CREATE TABLE dl2.bbox AS
SELECT id, geom, xmin, ymin, xmax, ymax FROM source ORDER BY spatial_sort;

-- Layout 3: cell-partitioned + bbox + Hilbert-sorted
ATTACH 'ducklake:$TMPDIR/l3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');
CREATE TABLE dl3.cell AS
SELECT id, geom, xmin, ymin, xmax, ymax, spatial_cell
FROM source ORDER BY spatial_sort;
ALTER TABLE dl3.cell SET PARTITIONED BY (spatial_cell);
" 2>&1 | tail -1

echo ">> Setup complete. Running queries..."
echo

# ---- Layout metrics ----
echo "=== Layout metrics ==="
echo "tier: $TIER"
echo "points: $N"

# File counts per layout
L1_FILES=$(find "$TMPDIR/l1/" -name "*.parquet" 2>/dev/null | wc -l)
L2_FILES=$(find "$TMPDIR/l2/" -name "*.parquet" 2>/dev/null | wc -l)
L3_FILES=$(find "$TMPDIR/l3/" -name "*.parquet" 2>/dev/null | wc -l)
echo "layout1_files: $L1_FILES"
echo "layout2_files: $L2_FILES"
echo "layout3_files: $L3_FILES"

# Cell cardinality (distinct partitions)
CELLS=$(duckdb -unsigned -csv -noheader -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/l3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');" \
  -c "SELECT count(DISTINCT spatial_cell) FROM dl3.cell;" 2>/dev/null)
echo "distinct_cells: $CELLS"

echo

# ---- Range query parity + pruning evidence ----
echo "=== Range query (distance < 10 from origin) ==="

L1_CNT=$(duckdb -unsigned -csv -noheader -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/l1.ducklake' AS dl1 (DATA_PATH '$TMPDIR/l1/');" \
  -c "SELECT count(*) FROM dl1.flat WHERE st_distance(geom, st_point(0.0, 0.0)) < 10.0;" 2>/dev/null)

L2_CNT=$(duckdb -unsigned -csv -noheader -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/l2.ducklake' AS dl2 (DATA_PATH '$TMPDIR/l2/');" \
  -c "SELECT count(*) FROM dl2.bbox WHERE xmax >= -10.0 AND xmin <= 10.0 AND ymax >= -10.0 AND ymin <= 10.0 AND st_distance(geom, st_point(0.0, 0.0)) < 10.0;" 2>/dev/null)

L3_CNT=$(duckdb -unsigned -csv -noheader -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/l3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');" \
  -c "SELECT count(*) FROM dl3.cell WHERE spatial_cell IN (SELECT quadkey FROM st_covering_quadkeys(st_makeenvelope(-10.0, -10.0, 10.0, 10.0), $QZOOM, 1000)) AND xmax >= -10.0 AND xmin <= 10.0 AND ymax >= -10.0 AND ymin <= 10.0 AND st_distance(geom, st_point(0.0, 0.0)) < 10.0;" 2>/dev/null)

L3_CAND=$(duckdb -unsigned -csv -noheader -cmd "LOAD '$EXT';" -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/l3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');" \
  -c "SELECT count(*) FROM dl3.cell WHERE spatial_cell IN (SELECT quadkey FROM st_covering_quadkeys(st_makeenvelope(-10.0, -10.0, 10.0, 10.0), $QZOOM, 1000));" 2>/dev/null)

echo "layout1_exact:       $L1_CNT"
echo "layout2_bbox+exact:  $L2_CNT"
echo "layout3_cell+bbox+e: $L3_CNT"
echo "layout3_candidates:  $L3_CAND (of $N total)"
echo "parity: $([ "$L1_CNT" = "$L2_CNT" ] && [ "$L2_CNT" = "$L3_CNT" ] && echo PASS || echo FAIL)"
echo "cell_pruning_ratio: $(echo "scale=2; $L3_CAND * 100 / $N" | bc 2>/dev/null || echo "N/A")%"

echo
echo ">> Harness complete."
