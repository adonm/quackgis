#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# End-to-end benchmark runner:
#   1. build the sedonadb extension
#   2. package it as a .duckdb_extension
#   3. generate SpatialBench parquet (trip SF 0.1, building SF 1)
#   4. set up a local DuckLake (DuckDB catalog + local data folder)
#   5. run the adapted SpatialBench queries with timing
#
# Prereqs: cargo, duckdb (1.5.x) on PATH; spatialbench-cli built at
#          .tmp/ref/sedona-spatialbench/target/release/spatialbench-cli
set -euo pipefail
cd "$(dirname "$0")/.."

EXT="build/dev/sedonadb.duckdb_extension"
SPATIALBENCH="${SPATIALBENCH:-.tmp/ref/sedona-spatialbench/target/release/spatialbench-cli}"

echo "==> build + package extension"
cargo build --release
./target/release/sedonadb-package target/release/libsedonadb.so "$EXT" linux_amd64

echo "==> generate SpatialBench parquet"
mkdir -p build/spatialbench-sf0.1 build/spatialbench-sf1-fragments
[ -f build/spatialbench-sf0.1/trip.parquet ] || \
  "$SPATIALBENCH" -s 0.1 --format=parquet -o build/spatialbench-sf0.1 --tables trip,customer,driver,vehicle
[ -f build/spatialbench-sf1-fragments/building.parquet ] || \
  "$SPATIALBENCH" -s 1 --format=parquet -o build/spatialbench-sf1-fragments --tables building

# Zone is expensive to generate (~156k complex polygons even at SF 0.1).
# Cache it: download one pre-generated partition from the SpatialBench HF dataset
# (reused on subsequent runs). Set SB_ZONE_PARQUET to point elsewhere, or
# SB_SKIP_ZONE=1 to skip zone-dependent queries.
SB_ZONE_PARQUET="${SB_ZONE_PARQUET:-build/spatialbench-sf0.1/zone/zone.1.parquet}"
if [ -z "${SB_SKIP_ZONE:-}" ] && [ ! -f "$SB_ZONE_PARQUET" ]; then
  echo "  caching zone (one HF partition, ~222 MB)..."
  mkdir -p "$(dirname "$SB_ZONE_PARQUET")"
  curl -fL --max-time 600 -o "$SB_ZONE_PARQUET" \
    "https://huggingface.co/datasets/apache-sedona/spatialbench/resolve/main/v0.1.0/sf0.1/zone/zone.1.parquet"
fi

echo "==> set up local DuckLake"
rm -rf build/lake && mkdir -p build/lake/data
duckdb -unsigned < benchmarks/setup_lake.sql

echo "==> run SpatialBench queries (timed)"
duckdb -unsigned < benchmarks/spatialbench_lake.sql
