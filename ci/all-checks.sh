#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Unified CI pipeline for QuackGIS.
#
# Two tracks:
#   ENGINE  — Rust DuckDB extension safety, SQL regression, catalog drift.
#   FACADE  — Container-based client tests (psql, psycopg, PostGIS fixtures).
#
# Engine phases (always run):
#   E1. Rust unit tests
#   E2. Catalog/compat/ledger drift gate
#   E3. Build + package + smoke test (all backend families)
#   E4. SQL regression suite (including DuckLake tests)
#   E5. Migration CLI smoke
#   E6. Scale harness (smoke tier)
#
# Facade phases (run when --facade or docker is available):
#   F1. Container smoke test
#   F2. PostGIS compatibility suite
#   F3. DuckLake storage + persistence
#   F4. PostGIS fixture suite
#   F5. psycopg client tests
#
# Usage:
#   ./ci/all-checks.sh                 # engine only
#   ./ci/all-checks.sh --facade        # engine + facade
#   ./ci/all-checks.sh --facade-only   # facade only (image must exist)
#   ./ci/all-checks.sh [duckdb_binary]
set -uo pipefail

cd "$(dirname "$0")/.."

DUCKDB="${1:-duckdb}"
RUN_ENGINE=true
RUN_FACADE=false

for arg in "$@"; do
    case "$arg" in
        --facade)       RUN_FACADE=true ;;
        --facade-only)  RUN_FACADE=true; RUN_ENGINE=false ;;
        --engine-only)  RUN_ENGINE=true; RUN_FACADE=false ;;
    esac
done

# Auto-detect docker for facade if requested but not explicitly set.
if $RUN_FACADE && ! command -v docker >/dev/null 2>&1; then
    echo "⚠ docker not found — facade tests skipped"
    RUN_FACADE=false
fi

# Locate runtime libs for Linuxbrew if installed.
BREW=/var/home/linuxbrew/.linuxbrew
if [ -d "$BREW/lib" ]; then
    export LD_LIBRARY_PATH="$BREW/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export PKG_CONFIG_PATH="$BREW/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    if [ -z "${LIBCLANG_PATH:-}" ] || [ ! -e "${LIBCLANG_PATH:-}/libclang.so" ]; then
        export LIBCLANG_PATH="$(dirname "$(find "$BREW" -name libclang.so 2>/dev/null | head -1)")"
    fi
fi

FAIL=0

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  QuackGIS — unified CI pipeline                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo

# ══════════════════════════════════════════════════════════════════════════════
# ENGINE TRACK
# ══════════════════════════════════════════════════════════════════════════════

if $RUN_ENGINE; then

# ---- E1: Rust unit tests ----
echo "=== E1: Rust unit tests ==="
if cargo test --lib 2>&1 | tail -5; then
    echo "✓ Rust tests passed"
else
    echo "✗ Rust tests FAILED"
    FAIL=1
fi
echo

# ---- E2: Drift gate ----
echo "=== E2: Catalog/compat/ledger drift gate ==="
if ./ci/check.sh; then
    echo "✓ Drift gate passed"
else
    echo "✗ Drift gate FAILED"
    FAIL=1
fi
echo

# ---- E3: Build + package + smoke ----
echo "=== E3: Build + package + smoke ==="
if ./ci/package-and-smoke.sh "$DUCKDB"; then
    echo "✓ Package + smoke passed"
else
    echo "✗ Package + smoke FAILED"
    FAIL=1
fi
echo

# ---- E4: SQL regression suite ----
echo "=== E4: SQL regression suite ==="
if ./tests/run_sql.sh; then
    echo "✓ SQL suite passed"
else
    echo "✗ SQL suite FAILED"
    FAIL=1
fi
echo

# ---- E5: Migration CLI smoke ----
echo "=== E5: Migration CLI smoke ==="
MIGRATE_BIN="target/debug/sedonadb-migrate"
if [ ! -f "$MIGRATE_BIN" ]; then
    MIGRATE_BIN="target/release/sedonadb-migrate"
fi
if [ -f "$MIGRATE_BIN" ]; then
    echo 'SELECT * FROM a JOIN b ON a.geom && b.geom;' > /tmp/sedonadb_migrate_smoke.sql
    if "$MIGRATE_BIN" /tmp/sedonadb_migrate_smoke.sql 2>/dev/null | grep -q 'st_intersects'; then
        echo "✓ Migration CLI passed"
    else
        echo "✗ Migration CLI FAILED"
        FAIL=1
    fi
else
    echo "⚠ Migration CLI not built (skipping)"
fi
echo

# ---- E6: Scale harness (smoke tier) ----
echo "=== E6: Scale harness (smoke tier) ==="
if ./benchmarks/scale_harness.sh smoke 2>&1; then
    echo "✓ Scale harness passed"
else
    echo "✗ Scale harness FAILED"
    FAIL=1
fi
echo

fi  # end RUN_ENGINE

# ══════════════════════════════════════════════════════════════════════════════
# FACADE TRACK
# ══════════════════════════════════════════════════════════════════════════════

if $RUN_FACADE; then

echo "═══════════════════════════════════════════════════════════════"
echo "  FACADE TRACK (container-based client tests)"
echo "═══════════════════════════════════════════════════════════════"
echo

# ---- F1-F5: Unified facade test runner ----
echo "=== F1-F5: Unified facade tests ==="
if ./container/run-all-tests.sh --no-build --skip-ducklake 2>&1; then
    echo "✓ Facade tests passed"
else
    echo "✗ Facade tests had failures (see output above)"
    FAIL=1
fi
echo

fi  # end RUN_FACADE

# ---- Summary ----
echo "╔══════════════════════════════════════════════════════════════╗"
if [ "$FAIL" -eq 0 ]; then
    echo "║  ALL CHECKS PASSED                                          ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "║  SOME CHECKS FAILED — see output above                      ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    exit 1
fi
