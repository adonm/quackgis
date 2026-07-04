#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# PostGIS regression test runner for QuackGIS.
#
# Runs upstream PostGIS core regress tests against a QuackGIS container
# and reports the pass rate. This is the primary compatibility metric.
#
# Usage:
#   ./tests/run-postgis-regress.sh                    # against running container
#   ./tests/run-postgis-regress.sh --report report.md # write detailed report
#
# Environment:
#   PG_HOST     default: 127.0.0.1
#   PG_PORT     default: 55432
#   PG_USER     default: postgres
#   PG_PASSWORD default: quackgis
#   PG_DB       default: postgres
set -uo pipefail

cd "$(dirname "$0")/.."

PG_HOST="${PG_HOST:-127.0.0.1}"
PG_PORT="${PG_PORT:-55432}"
PG_USER="${PG_USER:-postgres}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_DB="${PG_DB:-postgres}"

export PGPASSWORD="$PG_PASSWORD"

TEST_DIR="tests/postgis-regress/core"
REPORT_FILE=""

for arg in "$@"; do
    case "$arg" in
        --report) shift; REPORT_FILE="${1:-build/postgis-regress-report.md}" ;;
    esac
done

TOTAL=0
PASS=0
FAIL=0
ERROR=0
RESULTS=()

# Sort test SQL files alphabetically.
TESTS=$(find "$TEST_DIR" -name '*.sql' | sort)

for sql_file in $TESTS; do
    test_name=$(basename "$sql_file" .sql)
    expected_file="$TEST_DIR/${test_name}_expected"

    # Skip if no expected file.
    if [ ! -f "$expected_file" ]; then
        continue
    fi

    TOTAL=$((TOTAL + 1))

    # Run the test SQL through psql. Capture stdout and stderr separately.
    # -tA: unaligned, no headers (matches PostGIS expected format)
    # -v ON_ERROR_STOP=0: continue on errors (collect all output)
    actual=$(psql -h "$PG_HOST" -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB" \
        -tA -v ON_ERROR_STOP=0 -f "$sql_file" 2>/dev/null || true)

    # Normalize: trim trailing whitespace from each line
    actual_normalized=$(echo "$actual" | sed 's/[[:space:]]*$//')
    expected_normalized=$(cat "$expected_file" | sed 's/[[:space:]]*$//')

    if [ "$actual_normalized" = "$expected_normalized" ]; then
        PASS=$((PASS + 1))
        RESULTS+=("PASS|$test_name")
    elif [ -z "$actual_normalized" ]; then
        ERROR=$((ERROR + 1))
        RESULTS+=("ERROR|$test_name|no output")
    else
        FAIL=$((FAIL + 1))
        # Capture first difference for the report
        first_diff=$(diff <(echo "$expected_normalized") <(echo "$actual_normalized") | head -3)
        RESULTS+=("FAIL|$test_name|$first_diff")
    fi
done

# Summary
RATE="N/A"
if [ "$TOTAL" -gt 0 ]; then
    RATE=$(echo "scale=1; $PASS * 100 / $TOTAL" | bc)
fi

echo
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  PostGIS Compatibility Test Results                         ║"
echo "║                                                             ║"
echo "║  Total:  $TOTAL                                             ║"
echo "║  Pass:   $PASS                                              ║"
echo "║  Fail:   $FAIL                                              ║"
echo "║  Error:  $ERROR                                             ║"
echo "║  Rate:   ${RATE}%                                           ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo

# Detailed report
if [ -n "$REPORT_FILE" ]; then
    mkdir -p "$(dirname "$REPORT_FILE")"
    {
        echo "# PostGIS Compatibility Report"
        echo
        echo "**Date:** $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "**Total:** $TOTAL"
        echo "**Pass:** $PASS"
        echo "**Fail:** $FAIL"
        echo "**Error:** $ERROR"
        echo "**Rate:** ${RATE}%"
        echo
        echo "| Status | Test | Detail |"
        echo "|---|---|---|"
        for r in "${RESULTS[@]}"; do
            status=$(echo "$r" | cut -d'|' -f1)
            name=$(echo "$r" | cut -d'|' -f2)
            detail=$(echo "$r" | cut -d'|' -f3- | head -1 | tr '|' ' ')
            echo "| $status | $name | $detail |"
        done
    } > "$REPORT_FILE"
    echo "Report: $REPORT_FILE"
fi

# List failures for quick triage
if [ "$FAIL" -gt 0 ] || [ "$ERROR" -gt 0 ]; then
    echo "Failing tests:"
    for r in "${RESULTS[@]}"; do
        status=$(echo "$r" | cut -d'|' -f1)
        if [ "$status" != "PASS" ]; then
            echo "  $r"
        fi
    done
fi
