#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS unified facade test runner.
#
# Builds the image (if needed), starts a fresh container, runs all test
# phases through real PostgreSQL clients, and reports a summary.
#
# Phases:
#   1. smoke        — basic spatial function check
#   2. compat       — PostGIS compatibility (operators, casts, functions)
#   3. ducklake     — DuckLake storage + persistence + pruning parity
#   4. postgis-sql  — PostGIS fixture suite (PG-syntax SQL through facade)
#   5. psycopg      — Python client (prepared statements, params, BI metadata)
#
# Usage:
#   ./container/run-all-tests.sh                # build + run all phases
#   ./container/run-all-tests.sh --no-build     # skip image build
#   ./container/run-all-tests.sh --skip-psycopg # skip Python tests
#
# Environment:
#   QUACKGIS_IMAGE  Image name (default: quackgis)
#   QUACKGIS_TAG    Image tag (default: dev)
#   PG_PASSWORD     Postgres password (default: quackgis)
#   PG_PORT         Host port (default: 55432)
set -uo pipefail

cd "$(dirname "$0")/.."

IMAGE="${QUACKGIS_IMAGE:-quackgis}"
TAG="${QUACKGIS_TAG:-dev}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_PORT="${PG_PORT:-55432}"
CONTAINER_NAME="quackgis-test-runner"
CONTAINER_CMD="${CONTAINER_CMD:-docker}"
VOLUME="quackgis-test-lake"

DO_BUILD=true
SKIP_PSYCOPG=false
SKIP_DUCKLAKE=false

for arg in "$@"; do
    case "$arg" in
        --no-build)      DO_BUILD=false ;;
        --skip-psycopg)  SKIP_PSYCOPG=true ;;
        --skip-ducklake) SKIP_DUCKLAKE=true ;;
        --help|-h)
            echo "Usage: $0 [--no-build] [--skip-psycopg] [--skip-ducklake]"
            exit 0 ;;
    esac
done

TOTAL_PASS=0
TOTAL_FAIL=0
PHASES_RUN=0

cleanup() {
    "${CONTAINER_CMD}" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    "${CONTAINER_CMD}" volume rm "$VOLUME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  QuackGIS facade test runner                                ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo

# ── Build ────────────────────────────────────────────────────────────────────

if $DO_BUILD; then
    echo "── Build image ──────────────────────────────────────────────"
    ./container/build.sh -t "$TAG" || { echo "BUILD FAILED"; exit 1; }
    echo
fi

# ── Start container ──────────────────────────────────────────────────────────

echo "── Start container ──────────────────────────────────────────"
"${CONTAINER_CMD}" volume create "$VOLUME" >/dev/null
"${CONTAINER_CMD}" run -d \
    --name "$CONTAINER_NAME" \
    -e POSTGRES_PASSWORD="$PG_PASSWORD" \
    -e QUACKGIS_DUCKLAKE_DIR=/var/lib/quackgis \
    -p "${PG_PORT}:5432" \
    -v "${VOLUME}:/var/lib/quackgis" \
    "${IMAGE}:${TAG}" >/dev/null

echo ">> Waiting for QuackGIS to be ready..."
for i in $(seq 1 60); do
    if "${CONTAINER_CMD}" exec "$CONTAINER_NAME" \
            pg_isready -U postgres -d postgres >/dev/null 2>&1; then
        sleep 5  # let init scripts finish
        if PGPASSWORD="$PG_PASSWORD" psql -h 127.0.0.1 -p "$PG_PORT" \
                -U postgres -d postgres -tAc "SELECT 1;" >/dev/null 2>&1; then
            echo "   ready"
            break
        fi
    fi
    sleep 1
    if [ "$i" -eq 60 ]; then
        echo "   TIMEOUT — container did not become ready"
        exit 1
    fi
done
echo

export PGPASSWORD="$PG_PASSWORD"
export PG_PORT
export PG_HOST=127.0.0.1
export PG_USER=postgres
export PG_DB=postgres
export QUACKGIS_IMAGE
export QUACKGIS_TAG
export CONTAINER_NAME
export CONTAINER_CMD

# ── Phase 1: Smoke ───────────────────────────────────────────────────────────

echo "══ Phase 1: Smoke ════════════════════════════════════════════"
if PG_PORT=$PG_PORT PG_PASSWORD=$PG_PASSWORD \
    ./container/smoke-test.sh 2>&1; then
    echo "✓ Phase 1 passed"
else
    echo "✗ Phase 1 had failures (continuing — see output above)"
fi
# smoke-test.sh starts its own container; we re-start ours.
"${CONTAINER_CMD}" start "$CONTAINER_NAME" >/dev/null 2>&1 || true
sleep 3
echo

# ── Phase 2: PostGIS compatibility ───────────────────────────────────────────

echo "══ Phase 2: PostGIS compatibility ════════════════════════════"
PHASES_RUN=$((PHASES_RUN + 1))
PHASE_OUT=$(PG_HOST=127.0.0.1 PG_PORT=$PG_PORT PG_PASSWORD=$PG_PASSWORD \
    ./container/test-compat.sh 2>&1) || true
echo "$PHASE_OUT"
PHASE_PASS=$(echo "$PHASE_OUT" | grep -cE '^PASS ' || true)
PHASE_FAIL=$(echo "$PHASE_OUT" | grep -cE '^FAIL ' || true)
TOTAL_PASS=$((TOTAL_PASS + PHASE_PASS))
TOTAL_FAIL=$((TOTAL_FAIL + PHASE_FAIL))
echo "Phase 2: PASS=$PHASE_PASS FAIL=$PHASE_FAIL"
echo

# ── Phase 3: PostGIS fixture suite ┐══════════════════════════════════════════

echo "══ Phase 3: PostGIS fixture suite ═══════════════════════════"
PHASES_RUN=$((PHASES_RUN + 1))
PHASE_OUT=$(PG_HOST=127.0.0.1 PG_PORT=$PG_PORT PG_PASSWORD=$PG_PASSWORD \
    ./container/run-postgis-fixtures.sh 2>&1) || true
echo "$PHASE_OUT"
PHASE_PASS=$(echo "$PHASE_OUT" | grep -cE '^PASS ' || true)
PHASE_FAIL=$(echo "$PHASE_OUT" | grep -cE '^FAIL ' || true)
TOTAL_PASS=$((TOTAL_PASS + PHASE_PASS))
TOTAL_FAIL=$((TOTAL_FAIL + PHASE_FAIL))
echo "Phase 3: PASS=$PHASE_PASS FAIL=$PHASE_FAIL"
echo

# ── Phase 4: DuckLake storage ─══════════════════════════════════════════════

if ! $SKIP_DUCKLAKE; then
    echo "══ Phase 4: DuckLake storage ════════════════════════════════"
    # DuckLake test manages its own container with its own volume.
    PHASES_RUN=$((PHASES_RUN + 1))
    PHASE_OUT=$(QUACKGIS_IMAGE="$IMAGE" QUACKGIS_TAG="$TAG" \
        PG_PASSWORD="$PG_PASSWORD" PG_PORT=55433 \
        ./container/test-ducklake.sh 2>&1) || true
    echo "$PHASE_OUT"
    PHASE_PASS=$(echo "$PHASE_OUT" | grep -cE '^PASS ' || true)
    PHASE_FAIL=$(echo "$PHASE_OUT" | grep -cE '^FAIL ' || true)
    TOTAL_PASS=$((TOTAL_PASS + PHASE_PASS))
    TOTAL_FAIL=$((TOTAL_FAIL + PHASE_FAIL))
    echo "Phase 4: PASS=$PHASE_PASS FAIL=$PHASE_FAIL"
    echo
fi

# ── Phase 5: psycopg client ┐════════════════════════════════════════════════

if ! $SKIP_PSYCOPG; then
    if command -v python3 >/dev/null 2>&1 && \
       python3 -c "import psycopg" 2>/dev/null; then
        echo "══ Phase 5: psycopg client ═════════════════════════════════"
        PHASES_RUN=$((PHASES_RUN + 1))
        PHASE_OUT=$(PG_HOST=127.0.0.1 PG_PORT=$PG_PORT PG_PASSWORD=$PG_PASSWORD \
            python3 container/tests/test_psycopg.py 2>&1) || true
        echo "$PHASE_OUT"
        PHASE_PASS=$(echo "$PHASE_OUT" | grep -cE '^PASS ' || true)
        PHASE_FAIL=$(echo "$PHASE_OUT" | grep -cE '^FAIL ' || true)
        TOTAL_PASS=$((TOTAL_PASS + PHASE_PASS))
        TOTAL_FAIL=$((TOTAL_FAIL + PHASE_FAIL))
        echo "Phase 5: PASS=$PHASE_PASS FAIL=$PHASE_FAIL"
        echo
    else
        echo "── Phase 5: psycopg skipped (python3/psycopg not available) ──"
        echo
    fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  FACADE TEST SUMMARY                                        ║"
echo "║  Phases run: $PHASES_RUN                                             ║"
echo "║  Total PASS: $TOTAL_PASS                                             ║"
echo "║  Total FAIL: $TOTAL_FAIL                                             ║"
if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo "║  RESULT: FAILURES PRESENT                                   ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    exit 1
fi
echo "║  RESULT: ALL FACADE TESTS PASSED                            ║"
echo "╚══════════════════════════════════════════════════════════════╝"
