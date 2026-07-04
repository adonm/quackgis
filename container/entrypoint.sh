#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS container entrypoint (M7+).
#
# Wraps the standard postgres docker-entrypoint. pg_ducklake uses native
# DuckLake table AM — no per-session ATTACH needed. This entrypoint:
#   1. Sets up DuckLake data directory from QUACKGIS_* env vars.
#   2. Writes the preload config (pg_ducklake must be preloaded).
#   3. Hands off to the standard postgres docker-entrypoint.

set -e

DUCKLAKE_DIR="${QUACKGIS_DUCKLAKE_DIR:-/var/lib/quackgis}"
mkdir -p "$DUCKLAKE_DIR" "$DUCKLAKE_DIR/data" 2>/dev/null || true

echo ">> QuackGIS DuckLake configuration:"
echo "   data dir:  ${DUCKLAKE_DIR}"
echo "   data path: ${QUACKGIS_DUCKLAKE_DATA_PATH:-${DUCKLAKE_DIR}/data/}"

exec /usr/local/bin/docker-entrypoint.sh "$@"
