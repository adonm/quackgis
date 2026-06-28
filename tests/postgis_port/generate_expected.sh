#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Generate expected outputs from a pinned PostGIS Docker container.
#
# This is OPTIONAL — case files carry hand-verified expected values for
# standard geometries.  The Docker generator is for full regression coverage
# and CI pipelines that have Docker available.
#
# Usage: ./tests/postgis_port/generate_expected.sh
#
# Requires: docker, psql (client or dockerized).
set -euo pipefail
cd "$(dirname "$0")"

POSTGIS_IMAGE="postgis/postgis:16-3.5"
CONTAINER="postgis-port-gen"
EXPECTED_DIR="expected"
mkdir -p "$EXPECTED_DIR"

echo ">> Starting PostGIS container ($POSTGIS_IMAGE)…"
docker run -d --name "$CONTAINER" -e POSTGRES_PASSWORD=port \
    -e POSTGRES_DB=portdb "$POSTGIS_IMAGE" >/dev/null
trap 'docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT

echo ">> Waiting for PostGIS…"
for i in $(seq 1 30); do
    if docker exec "$CONTAINER" pg_isready -U postgres -d portdb >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

docker exec "$CONTAINER" psql -U postgres -d portdb \
    -c "CREATE EXTENSION IF NOT EXISTS postgis;" >/dev/null

echo ">> Running PostGIS source SQL from case files…"
for casefile in cases/*.sql; do
    name=$(basename "$casefile" .sql)
    # Extract lines starting with "-- PG:" and strip the prefix.
    pg_sql=$(grep '^-- PG:' "$casefile" | sed 's/^-- PG: //')
    if [ -z "$pg_sql" ]; then
        continue
    fi
    echo "  $name…"
    # Run each PG statement and capture output.
    while IFS= read -r line; do
        if [ -z "$line" ]; then continue; fi
        result=$(docker exec "$CONTAINER" psql -U postgres -d portdb -t -A -c "$line" 2>&1 || echo "ERROR")
        echo "  $line => $result"
    done <<< "$pg_sql"
done

echo ">> Done."
