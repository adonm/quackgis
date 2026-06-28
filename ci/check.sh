#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# CI drift gate: verify that the live registry matches committed docs and that
# the SedonaDB bridge inventory has no unclassified or undocumented routing
# decisions. Run before every commit/push.
#
# Usage: ./ci/check.sh
# Exits non-zero on any drift.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Catalog drift check ==="
python3 tools/catalog_audit.py --check

echo "=== Compat check (bridge routing) ==="
python3 tools/catalog_audit.py --compat-check

echo "=== Ledger freshness check ==="
# Regenerate the ledger and verify it matches the committed version.
cp docs/SEDONA_LEDGER.md /tmp/sedona_ledger_committed.md
python3 tools/catalog_audit.py --generate-ledger >/dev/null
if ! diff -q docs/SEDONA_LEDGER.md /tmp/sedona_ledger_committed.md >/dev/null; then
    echo "FAIL: docs/SEDONA_LEDGER.md is stale. Run:"
    echo "  python3 tools/catalog_audit.py --generate-ledger"
    echo "  git add docs/SEDONA_LEDGER.md"
    cp /tmp/sedona_ledger_committed.md docs/SEDONA_LEDGER.md
    exit 1
fi
echo "Ledger is up to date."
rm -f /tmp/sedona_ledger_committed.md

echo "=== JSON export freshness check ==="
cp docs/sedonadb_compat.json /tmp/sedona_json_committed.json
python3 tools/catalog_audit.py --export-json >/dev/null
if ! diff -q docs/sedonadb_compat.json /tmp/sedona_json_committed.json >/dev/null; then
    echo "FAIL: docs/sedonadb_compat.json is stale. Run:"
    echo "  python3 tools/catalog_audit.py --export-json"
    echo "  git add docs/sedonadb_compat.json"
    cp /tmp/sedona_json_committed.json docs/sedonadb_compat.json
    exit 1
fi
echo "JSON export is up to date."
rm -f /tmp/sedona_json_committed.json

echo "=== All drift gates passed ==="
