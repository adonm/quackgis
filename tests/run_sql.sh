#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Run every DuckDB SQL regression in tests/ against a packaged extension and
# report a clean PASS/FAIL summary.
#
# M27 hardening: non-zero DuckDB exits (including SIGSEGV = 139, SIGABRT = 134)
# are now counted as CRASH failures, not silently swallowed.
#
# Usage:
#   ./tests/run_sql.sh [path/to/sedonadb.duckdb_extension] [duckdb_binary]
set -uo pipefail

cd "$(dirname "$0")/.."

EXT="${1:-build/dev/sedonadb.duckdb_extension}"
DUCKDB="${2:-duckdb}"

if [ -d /var/home/linuxbrew/.linuxbrew/lib ]; then
    export LD_LIBRARY_PATH="/var/home/linuxbrew/.linuxbrew/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

if [ ! -f "$EXT" ]; then
    echo "FATAL: extension not found at $EXT" >&2; exit 2
fi

# ── Test file lists by risk tier ──────────────────────────────────────
# Tier A: stateless deterministic SQL (the bulk of regression coverage)
SQL_FILES_A=(tests/all_functions.sql tests/sedona_bridge.sql tests/vector_encodings.sql tests/fidelity.sql tests/edge_cases.sql tests/raster.sql tests/reference/postgis_compat.sql tests/reference/m1_fixtures.sql tests/reference/m2_fixtures.sql tests/reference/m3_fixtures.sql tests/reference/m4_fixtures.sql tests/reference/m5_fixtures.sql tests/reference/m6_fixtures.sql tests/reference/m7_fixtures.sql tests/postgis_port/cases/01_constructors.sql tests/postgis_port/cases/02_accessors.sql tests/postgis_port/cases/03_predicates.sql tests/postgis_port/cases/04_overlay.sql tests/postgis_port/cases/05_validity.sql tests/postgis_port/cases/06_dump.sql tests/postgis_port/cases/07_line_editing.sql tests/postgis_port/cases/08_operator_rewrites.sql tests/reference/m9_fixtures.sql tests/reference/m11_fixtures.sql tests/reference/m14_fixtures.sql tests/reference/m15_fixtures.sql tests/reference/m16_fixtures.sql tests/reference/m22_fixtures.sql tests/reference/m23_fixtures.sql tests/reference/m23b_fixtures.sql tests/reference/m25_fixtures.sql tests/reference/delta_closure_fixtures.sql tests/reference/m28_fixtures.sql tests/reference/m29_fixtures.sql tests/upstream_curated/postgis_relate.sql tests/upstream_curated/postgis_boundary.sql tests/upstream_curated/postgis_simplify.sql tests/upstream_curated/postgis_empty.sql tests/upstream_curated/postgis_measures.sql tests/upstream_curated/postgis_topology.sql tests/upstream_curated/postgis_editing.sql tests/upstream_curated/postgis_accessors.sql tests/upstream_curated/postgis_affine.sql tests/upstream_curated/postgis_dump.sql tests/upstream_curated/postgis_processing.sql tests/upstream_curated/postgis_predicates.sql)
# Tier B: macro-dependent (requires optional DuckLake spatial macros)
SQL_FILES_B=(tests/reference/m19_fixtures.sql)
# Tier C: DuckLake stateful (requires ducklake extension, catalog cleanup)
SQL_FILES_C=(tests/reference/m10_ducklake.sql tests/reference/m17_scale.sql)

TOTAL_PASS=0
TOTAL_FAIL=0
FAILED_FILES=()

# Run a single SQL file and accumulate PASS/FAIL counts.
# Detects crashes via exit code (139=SIGSEGV, 134=SIGABRT, etc).
run_sql_file() {
    local file="$1"
    shift  # remaining args are duckdb -cmd flags
    local out exit_code
    out=$("$DUCKDB" -unsigned "$@" < "$file" 2>&1)
    exit_code=$?

    local pass fail crash
    pass=$(printf '%s\n' "$out" | grep -E '^PASS' | grep -vE 'CASE|THEN|ELSE' | wc -l)
    fail=$(printf '%s\n' "$out" | grep -E '^FAIL' | grep -vE 'CASE|THEN|ELSE' | wc -l)
    fail=$((fail + $(printf '%s\n' "$out" | grep -ciE '^(Error|Binder Error|Catalog Error|Runtime Error|Parser Error|Internal Error)' || true)))

    # Crash detection: non-zero exit that isn't a normal SQL error
    if [ "$exit_code" -ne 0 ]; then
        case "$exit_code" in
            139) crash="SIGSEGV" ;;
            134) crash="SIGABRT" ;;
            *)   crash="exit=$exit_code" ;;
        esac
        # Only count as crash if there are no explicit error lines already
        if [ "$fail" -eq 0 ]; then
            fail=1
            printf '%-48s  PASS=%-3d  CRASH=%s\n' "$file" "$pass" "$crash"
            TOTAL_PASS=$((TOTAL_PASS + pass))
            TOTAL_FAIL=$((TOTAL_FAIL + fail))
            FAILED_FILES+=("$file")
            return
        fi
    fi

    printf '%-48s  PASS=%-3d  FAIL=%-3d\n' "$file" "$pass" "$fail"
    TOTAL_PASS=$((TOTAL_PASS + pass))
    TOTAL_FAIL=$((TOTAL_FAIL + fail))
    [ "$fail" -gt 0 ] && FAILED_FILES+=("$file")
}

# ── Tier A: Standard SQL tests ────────────────────────────────────────
echo "Tier A: Standard SQL tests"
for f in "${SQL_FILES_A[@]}"; do
    run_sql_file "$f" -cmd "LOAD '$(pwd)/$EXT';"
done
echo "------------------------------------------------"
printf 'TOTAL (Tier A)                   PASS=%-3d  FAIL=%-3d\n' "$TOTAL_PASS" "$TOTAL_FAIL"

# ── Tier B: Macro-dependent tests ─────────────────────────────────────
if [ "$TOTAL_FAIL" -eq 0 ] && [ ${#SQL_FILES_B[@]} -gt 0 ]; then
    echo
    echo "Tier B: Macro-dependent SQL tests:"
    for f in "${SQL_FILES_B[@]}"; do
        run_sql_file "$f" -cmd "LOAD '$(pwd)/$EXT';" -cmd ".read sql/ducklake_spatial_macros.sql"
    done
    echo "------------------------------------------------"
    printf 'TOTAL (incl Tier B)              PASS=%-3d  FAIL=%-3d\n' "$TOTAL_PASS" "$TOTAL_FAIL"
fi

# ── Tier C: DuckLake stateful tests ───────────────────────────────────
if [ "$TOTAL_FAIL" -eq 0 ] && [ ${#SQL_FILES_C[@]} -gt 0 ]; then
    echo
    echo "Tier C: DuckLake SQL tests:"
    for f in "${SQL_FILES_C[@]}"; do
        run_sql_file "$f" -cmd "LOAD '$(pwd)/$EXT';" -cmd "LOAD ducklake;"
        rm -rf ':memory::' ':memory::.files' 2>/dev/null || true
    done
    echo "------------------------------------------------"
    printf 'TOTAL (incl Tier C)              PASS=%-3d  FAIL=%-3d\n' "$TOTAL_PASS" "$TOTAL_FAIL"
fi

# ── Result ────────────────────────────────────────────────────────────
if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo "FAILED files: ${FAILED_FILES[*]}"
    exit 1
fi
echo "ALL SQL REGRESSIONS PASSED"
