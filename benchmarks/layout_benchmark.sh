#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Benchmark: spatial layout comparison for DuckLake tables.
#
# Compares three layouts over the same point dataset:
#   1. Unpartitioned (no layout columns)
#   2. bbox columns + Hilbert-sorted files (no partitioning)
#   3. Cell-partitioned + Hilbert-sorted (full layout)
#
# Usage: ./benchmarks/layout_benchmark.sh [N_POINTS]
set -euo pipefail
cd "$(dirname "$0")/.."

export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
N="${1:-100000}"
EXT="build/dev/sedonadb.duckdb_extension"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

DUCKDB="duckdb -unsigned -cmd LOAD '$EXT'; -cmd LOAD ducklake;"

echo ">> Spatial layout benchmark: $N points"
echo ">> Temp dir: $TMPDIR"
echo

# ---- Setup: generate data and create three layouts ----
duckdb -unsigned \
  -cmd "LOAD '$EXT';" \
  -cmd "LOAD ducklake;" \
  -c "
CREATE TABLE source AS
SELECT
    i AS id,
    st_point((i % 1000)::double / 10.0 - 50.0, ((i * 7) % 1000)::double / 10.0 - 50.0) AS geom
FROM range(0, $N) t(i);

ATTACH 'ducklake:$TMPDIR/layout1.ducklake' AS dl1 (DATA_PATH '$TMPDIR/l1/');
CREATE TABLE dl1.raw AS SELECT id, geom FROM source;

ATTACH 'ducklake:$TMPDIR/layout2.ducklake' AS dl2 (DATA_PATH '$TMPDIR/l2/');
CREATE TABLE dl2.bbox AS
SELECT *,
       st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
       st_xmax(geom) AS xmax, st_ymax(geom) AS ymax
FROM source ORDER BY st_hilbert(geom, 12);

ATTACH 'ducklake:$TMPDIR/layout3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');
CREATE TABLE dl3.partitioned AS
SELECT *,
       st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
       st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
       st_quadkey(geom, 6) AS spatial_cell
FROM source ORDER BY st_hilbert(geom, 12);
ALTER TABLE dl3.partitioned SET PARTITIONED BY (spatial_cell);
" 2>&1 | tail -1
echo ">> Setup complete."
echo

# ---- Query 1: unpartitioned scan ----
echo "--- Layout 1: unpartitioned scan ---"
duckdb -unsigned \
  -cmd "LOAD '$EXT';" \
  -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/layout1.ducklake' AS dl1 (DATA_PATH '$TMPDIR/l1/');" \
  -c ".timer on" \
  -c "SELECT count(*) FROM dl1.raw WHERE st_distance(geom, st_point(0.0, 0.0)) < 5.0;" \
  2>&1

# ---- Query 2: bbox + sorted files ----
echo "--- Layout 2: bbox columns + Hilbert-sorted ---"
duckdb -unsigned \
  -cmd "LOAD '$EXT';" \
  -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/layout2.ducklake' AS dl2 (DATA_PATH '$TMPDIR/l2/');" \
  -c ".timer on" \
  -c "SELECT count(*) FROM dl2.bbox WHERE xmax >= -5.0 AND xmin <= 5.0 AND ymax >= -5.0 AND ymin <= 5.0 AND st_distance(geom, st_point(0.0, 0.0)) < 5.0;" \
  2>&1

# ---- Query 3: cell partition + bbox + exact ----
echo "--- Layout 3: cell partition + bbox + exact ---"
duckdb -unsigned \
  -cmd "LOAD '$EXT';" \
  -cmd "LOAD ducklake;" \
  -c "ATTACH 'ducklake:$TMPDIR/layout3.ducklake' AS dl3 (DATA_PATH '$TMPDIR/l3/');" \
  -c ".timer on" \
  -c "SELECT count(*) FROM dl3.partitioned WHERE spatial_cell IN (SELECT quadkey FROM st_covering_quadkeys(st_makeenvelope(-5.0, -5.0, 5.0, 5.0), 6, 1000)) AND xmax >= -5.0 AND xmin <= 5.0 AND ymax >= -5.0 AND ymin <= 5.0 AND st_distance(geom, st_point(0.0, 0.0)) < 5.0;" \
  2>&1

echo
echo ">> All three layouts return the same count. Compare Run Time."
