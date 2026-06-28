#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Unified CI pipeline: run every quality gate in sequence. This is the
# canonical "does the project pass?" command. Run before every commit/push.
#
# Phases:
#   1. Rust unit tests
#   2. Catalog/compat/ledger drift gate
#   3. Build + package + smoke test (all backend families)
#   4. SQL regression suite (including DuckLake tests)
#   5. Migration CLI smoke (sedonadb-migrate binary)
#   6. Scale harness smoke tier (DuckLake pruning evidence)
#
# Phase 3 runs before the SQL suite because it (re)builds
# build/dev/sedonadb.duckdb_extension — the artifact phases 4 and 5 load.
# Running SQL first would silently test a stale extension.
#
# Usage: ./ci/all-checks.sh [duckdb_binary]
# Exits non-zero on any failure.
set -euo pipefail

cd "$(dirname "$0")/.."

DUCKDB="${1:-duckdb}"

# Locate runtime libs (libgdal/libproj/libgeos) for Linuxbrew if installed.
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
echo "║  sedonadb — unified CI pipeline                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo

# ---- Phase 1: Rust unit tests ----
echo "=== Phase 1: Rust unit tests ==="
if cargo test --lib 2>&1 | tail -5; then
    echo "✓ Rust tests passed"
else
    echo "✗ Rust tests FAILED"
    FAIL=1
fi
echo

# ---- Phase 2: Drift gate ----
echo "=== Phase 2: Catalog/compat/ledger drift gate ==="
if ./ci/check.sh; then
    echo "✓ Drift gate passed"
else
    echo "✗ Drift gate FAILED"
    FAIL=1
fi
echo

# ---- Phase 3: Build + package + smoke ----
echo "=== Phase 3: Build + package + smoke ==="
if ./ci/package-and-smoke.sh "$DUCKDB"; then
    echo "✓ Package + smoke passed"
else
    echo "✗ Package + smoke FAILED"
    FAIL=1
fi
echo

# ---- Phase 4: SQL regression suite ----
echo "=== Phase 4: SQL regression suite ==="
if ./tests/run_sql.sh; then
    echo "✓ SQL suite passed"
else
    echo "✗ SQL suite FAILED"
    FAIL=1
fi
echo

# ---- Phase 5: Migration CLI smoke ----
echo "=== Phase 5: Migration CLI smoke ==="
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

# ---- Phase 6: Scale harness (smoke tier) ----
echo "=== Phase 6: Scale harness (smoke tier) ==="
if ./benchmarks/scale_harness.sh smoke 2>&1; then
    echo "✓ Scale harness passed"
else
    echo "✗ Scale harness FAILED"
    FAIL=1
fi
echo

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
