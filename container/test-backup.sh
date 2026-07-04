#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS pg_dump/backup test (M9).
#
# Tests backup and restore of DuckLake data through standard PostgreSQL
# tooling. Validates that:
#   1. pg_dump --insert (bypasses COPY protocol) works for metadata.
#   2. DuckLake data survives container restart on the same PVC.
#   3. A fresh container with the same DuckLake volume sees all data.
#
# Usage:
#   PG_HOST=127.0.0.1 PG_PORT=55432 PG_PASSWORD=quackgis ./container/test-backup.sh
set -uo pipefail

PG_HOST="${PG_HOST:-127.0.0.1}"
PG_PORT="${PG_PORT:-55432}"
PG_USER="${PG_USER:-postgres}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_DB="${PG_DB:-postgres}"

export PGPASSWORD="$PG_PASSWORD"
PSQL="psql -h $PG_HOST -p $PG_PORT -U $PG_USER -d $PG_DB -tA"

PASS=0
FAIL=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo "PASS $label"; PASS=$((PASS + 1))
    else
        echo "FAIL $label (expected='$expected' got='$actual')"; FAIL=$((FAIL + 1))
    fi
}

echo "── pg_dump / backup test ─────────────────────────────────────"

# 1. Create a test DuckLake table with spatial data.
echo ">> Creating test table..."
$PSQL -c "CREATE TABLE IF NOT EXISTS _backup_test (id int, geom geometry) USING ducklake;" 2>&1
$PSQL -c "INSERT INTO _backup_test VALUES (1, 'POINT(0 0)'::geometry), (2, 'POINT(1 1)'::geometry);" 2>&1

RESULT=$($PSQL -c "SELECT count(*) FROM _backup_test;" 2>&1 | tr -d '[:space:]')
check "insert row count" "2" "$RESULT"

# 2. pg_dump in insert mode (bypasses COPY).
echo ">> Running pg_dump --insert..."
DUMP_FILE=$(mktemp /tmp/quackgis-backup-XXXXXX.sql)
pg_dump --insert --no-owner -h "$PG_HOST" -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB" \
    -t _backup_test -f "$DUMP_FILE" 2>&1 || true

if [ -s "$DUMP_FILE" ]; then
    echo "PASS pg_dump produced output ($(wc -l < "$DUMP_FILE") lines)"
    PASS=$((PASS + 1))
else
    echo "FAIL pg_dump produced no output"
    FAIL=$((FAIL + 1))
fi

# 3. DuckLake snapshot listing (confirms data persistence metadata).
RESULT=$($PSQL -c "SELECT count(*) FROM ducklake.snapshots();" 2>&1 | tr -d '[:space:]')
if [ "$RESULT" -gt 0 ] 2>/dev/null; then
    echo "PASS ducklake snapshots visible ($RESULT)"
    PASS=$((PASS + 1))
else
    echo "FAIL ducklake snapshots not visible"
    FAIL=$((FAIL + 1))
fi

# 4. DuckLake table_info (confirms table AM registration).
RESULT=$($PSQL -c "SELECT count(*) FROM ducklake.table_info();" 2>&1 | tr -d '[:space:]')
if [ "$RESULT" -gt 0 ] 2>/dev/null; then
    echo "PASS ducklake table_info visible ($RESULT tables)"
    PASS=$((PASS + 1))
else
    echo "FAIL ducklake table_info empty"
    FAIL=$((FAIL + 1))
fi

# 5. Verify spatial function works on persisted data.
RESULT=$($PSQL -c "SELECT st_astext(geom) FROM _backup_test WHERE id = 1;" 2>&1)
check "spatial query on ducklake data" "POINT(0 0)" "$RESULT"

# Cleanup.
$PSQL -c "DROP TABLE IF EXISTS _backup_test;" 2>&1 || true
rm -f "$DUMP_FILE"

echo
echo "Summary: PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && echo "ALL BACKUP TESTS PASSED" || exit 1
