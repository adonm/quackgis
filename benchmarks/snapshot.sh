#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SpatialBench snapshot: run the heavy workload tier and emit a structured
# markdown report with row counts and timings.
#
# Usage:
#   ./benchmarks/snapshot.sh [--skip-build] [--skip-zone]
#
# Output: benchmarks/snapshot-latest.md (also printed to stdout)
#
# This is the release QA harness described in ROADMAP.md Month 5/6. It is a
# manual/nightly gate, not required per-commit.
set -euo pipefail
cd "$(dirname "$0")/.."

EXT="build/dev/sedonadb.duckdb_extension"
SKIP_BUILD=""
SKIP_ZONE=""

for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --skip-zone)  SKIP_ZONE=1 ;;
  esac
done

# Locate runtime libs.
if [ -d /var/home/linuxbrew/.linuxbrew/lib ]; then
    export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

# --- Build + package ---------------------------------------------------
if [ -z "$SKIP_BUILD" ]; then
  echo "==> build + package" >&2
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/var/home/linuxbrew/.linuxbrew/lib/pkgconfig}"
  export LIBCLANG_PATH="${LIBCLANG_PATH:-/var/home/linuxbrew/.linuxbrew/Cellar/llvm/22.1.8/lib}"
  cargo build --release 2>&1 | tail -1 >&2
  ./target/release/sedonadb-package target/release/libsedonadb.so "$EXT" linux_amd64 >&2
fi

# --- Metadata ----------------------------------------------------------
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
DUCKDB_VER=$(duckdb -version 2>/dev/null | head -1 || echo "unknown")
MACHINE=$(uname -srm 2>/dev/null || echo "unknown")

# --- Run SpatialBench queries ------------------------------------------
# Reuse the existing run.sh infrastructure if data exists; otherwise fall
# back to perf_budget.sql.
SB_DIR="build/spatialbench-sf0.1"
HAS_SB_DATA=""

if [ -f "$SB_DIR/trip.parquet" ]; then
  HAS_SB_DATA=1
fi

{
echo "# SpatialBench snapshot"
echo
echo "| Field | Value |"
echo "|-------|-------|"
echo "| Date | $DATE |"
echo "| Extension commit | \`$COMMIT\` |"
echo "| DuckDB | $DUCKDB_VER |"
echo "| Platform | $MACHINE |"
echo "| Scale | SF 0.1 |"
echo

if [ -n "$HAS_SB_DATA" ]; then
  echo "## SpatialBench queries (timed)"
  echo
  echo '```'
  duckdb -unsigned -cmd "LOAD '$(pwd)/$EXT';" < benchmarks/spatialbench_lake.sql 2>&1 || echo "(some queries failed)"
  echo '```'
else
  echo "## Performance budget (SpatialBench data not available)"
  echo
  echo "Run \`./benchmarks/run.sh\` first to generate SpatialBench data."
  echo "Falling back to \`perf_budget.sql\`:"
  echo
  echo '```'
  duckdb -unsigned -cmd "LOAD '$(pwd)/$EXT';" < benchmarks/perf_budget.sql 2>&1 || echo "(some queries failed)"
  echo '```'
fi

echo
echo "## Catalog"
echo
echo '```'
python3 tools/catalog_audit.py 2>/dev/null || echo "(catalog audit unavailable)"
echo '```'
} | tee benchmarks/snapshot-latest.md

echo >&2
echo "==> snapshot written to benchmarks/snapshot-latest.md" >&2
