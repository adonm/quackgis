#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Scale report generator: runs the scale harness at multiple tiers, collects
# results, and emits a structured Markdown report suitable for release QA.
#
# This is the repeatable release evidence script. Exact-result parity is the
# only correctness oracle; timing is informational.
#
# Usage: ./benchmarks/scale_report.sh [output_file]
#   output_file: defaults to stdout
set -euo pipefail
cd "$(dirname "$0")/.."

EXT="build/dev/sedonadb.duckdb_extension"
OUT="${1:-/dev/stdout}"

if [ ! -f "$EXT" ]; then
    echo "ERROR: $EXT not found. Run 'cargo build --release && ./target/release/sedonadb-package ...' first." >&2
    exit 1
fi

if ! command -v duckdb &>/dev/null; then
    echo "ERROR: duckdb not found in PATH." >&2
    exit 1
fi

# Locate runtime libs.
if [ -d /var/home/linuxbrew/.linuxbrew/lib ]; then
    export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

TIERS="smoke local"
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

echo ">> Generating scale report (tiers: $TIERS)" >&2

# Collect results per tier
RESULTS=""
OVERALL_PARITY="PASS"

for TIER in $TIERS; do
    echo ">> Running tier: $TIER" >&2
    RAW=$(./benchmarks/scale_harness.sh "$TIER" 2>&1)

    POINTS=$(echo "$RAW" | grep '^points:' | awk '{print $2}')
    CELLS=$(echo "$RAW" | grep '^distinct_cells:' | awk '{print $2}')
    L1_FILES=$(echo "$RAW" | grep '^layout1_files:' | awk '{print $2}')
    L2_FILES=$(echo "$RAW" | grep '^layout2_files:' | awk '{print $2}')
    L3_FILES=$(echo "$RAW" | grep '^layout3_files:' | awk '{print $2}')
    L1_CNT=$(echo "$RAW" | grep '^layout1_exact:' | awk '{print $2}')
    L2_CNT=$(echo "$RAW" | grep '^layout2_bbox' | awk '{print $2}')
    L3_CNT=$(echo "$RAW" | grep '^layout3_cell' | awk '{print $2}')
    L3_CAND=$(echo "$RAW" | grep '^layout3_candidates:' | awk '{print $2}')
    PARITY=$(echo "$RAW" | grep '^parity:' | awk '{print $2}')
    RATIO=$(echo "$RAW" | grep '^cell_pruning_ratio:' | awk '{print $2}')

    if [ "$PARITY" != "PASS" ]; then
        OVERALL_PARITY="FAIL"
    fi

    RESULTS="${RESULTS}
| \`$TIER\` | $POINTS | $CELLS | $L1_CNT | $L2_CNT | $L3_CNT | $L3_CAND | $RATIO | $PARITY |"
done

# Object-size estimate (smoke tier, layout 3)
OBJ_BYTES=$(du -sb /tmp/scale_report_tmp_$$ 2>/dev/null | awk '{print $1}' || echo "N/A")

cat <<EOF > "$OUT"
# Scale validation report

Generated: $DATE
Commit: $COMMIT
Extension: $EXT

## Parity summary

Overall parity: **$OVERALL_PARITY**

All three layouts (flat / bbox+sorted / cell+bbox+sorted) must return identical
exact-result counts. Parity = correctness. Pruning ratio is informational.

## Tier comparison

| Tier | Points | Cells | L1 exact | L2 bbox+exact | L3 cell+bbox+exact | L3 candidates | Pruning ratio | Parity |
|---|---|---|---|---|---|---|---|---|
$RESULTS

## Interpretation

- **L1 = L2 = L3**: exact-result parity holds across all layouts and tiers.
- **Pruning ratio** = L3 candidates / total rows. Lower is better. Dense data
  yields smaller cells and better pruning.
- **File counts** grow with row count; target 100 MB–1 GB per Parquet object
  using \`ST_EstimatePartitionCount\`.

## Object-store deployment guidance

| Factor | Recommendation |
|---|---|
| Object size | 100 MB–1 GB per Parquet file |
| Partition key | \`ST_QuadKey(geom, zoom)\` at a zoom where cells have 100k–1M rows |
| Sort key | \`ST_Hilbert(geom, bits)\` to cluster files spatially |
| Catalog | DuckLake file catalog for single-writer; PostgreSQL for concurrent |
| Adaptive spec | Use sort-then-pack (M11) for skewed distributions |
| Query pattern | Three-stage: cell IN → bbox overlap → exact predicate |

### Choosing the zoom level

\`\`\`sql
-- Estimate partition count for target object size
SELECT st_estimatepartitioncount(
    total_rows := 1000000000,      -- 1 billion rows
    avg_row_bytes := 200,           -- ~200 bytes/row including geometry
    target_object_bytes := 536870912 -- 512 MB target
);
-- Returns ~376 partitions → use zoom 8-9 for quadkey

-- Verify: each partition should have 100k–10M rows
SELECT st_recommendzoom(376);  -- → 8 or 9
\`\`\`

### Monitoring pruning effectiveness

In production, query \`ducklake_file_column_stats\` to verify zone maps are
effective:

\`\`\`sql
-- Check bbox column stats for a spatial table
SELECT file_path, column_name, min_value, max_value
FROM ducklake_file_column_stats('dl.parcels')
WHERE column_name IN ('xmin', 'xmax', 'ymin', 'ymax')
ORDER BY file_path;
\`\`\`

If \`xmin\` and \`xmax\` ranges are very wide (spanning the entire dataset),
files are not spatially clustered — re-sort by \`ST_Hilbert\` and rewrite.

## Known limitations

- \`ST_CoveringQuadKeys\` cannot take lateral column arguments (DuckDB table
  function limitation). Pre-compute covering cells into a temp table for joins.
- DuckLake file catalog serializes writers. Use PostgreSQL catalog for
  concurrent writers.
- Timing is informational, not a correctness oracle.
EOF

echo ">> Scale report written to $OUT" >&2
echo ">> Overall parity: $OVERALL_PARITY" >&2

if [ "$OVERALL_PARITY" != "PASS" ]; then
    exit 1
fi
