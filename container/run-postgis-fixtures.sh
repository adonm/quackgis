#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# PostGIS fixture runner: runs PG-syntax spatial SQL through the QuackGIS
# facade and counts PASS/FAIL results.
#
# Each fixture file uses the same CASE WHEN ... THEN 'PASS ...' ELSE 'FAIL ...'
# convention as the engine-level SQL tests, but uses PostgreSQL syntax that
# the facade must parse (geometry casts, operators, PostGIS function names).
#
# Usage:
#   ./container/run-postgis-fixtures.sh
set -uo pipefail

PG_HOST="${PG_HOST:-127.0.0.1}"
PG_PORT="${PG_PORT:-55432}"
PG_USER="${PG_USER:-postgres}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_DB="${PG_DB:-postgres}"

export PGPASSWORD="$PG_PASSWORD"

FIXTURE_DIR="$(cd "$(dirname "$0")" && pwd)/tests/postgis-fixtures"

if [ ! -d "$FIXTURE_DIR" ]; then
    echo "No PostGIS fixtures found at $FIXTURE_DIR"
    exit 0
fi

TOTAL_PASS=0
TOTAL_FAIL=0

for f in "$FIXTURE_DIR"/*.sql; do
    [ -f "$f" ] || continue
    fname=$(basename "$f")
    out=$(psql -h "$PG_HOST" -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB" \
          -tA -f "$f" 2>&1) || true

    pass=$(echo "$out" | grep -cE '^PASS ' || true)
    fail=$(echo "$out" | grep -cE '^FAIL ' || true)
    err=$(echo "$out" | grep -ciE '^(ERROR|FATAL)' || true)
    fail=$((fail + err))

    TOTAL_PASS=$((TOTAL_PASS + pass))
    TOTAL_FAIL=$((TOTAL_FAIL + fail))

    if [ "$fail" -gt 0 ]; then
        printf '%-40s  PASS=%-3d  FAIL=%-3d\n' "$fname" "$pass" "$fail"
        echo "$out" | grep -E '^(FAIL|ERROR)' | head -5
    else
        printf '%-40s  PASS=%-3d\n' "$fname" "$pass"
    fi
done

echo
echo "PostGIS fixtures: PASS=$TOTAL_PASS FAIL=$TOTAL_FAIL"
if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi
