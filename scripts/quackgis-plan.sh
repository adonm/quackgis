#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-compose.sh"

"$compose" exec -T postgres psql \
    -X \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis <<'SQL'
EXPLAIN (VERBOSE, COSTS OFF)
SELECT id
FROM public.features_unbounded
WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326);
SQL

cat <<'EOF'

P2 remains open: the current WKT/PostGIS expression is evaluated locally.
The accepted implementation must use native WKB/EWKB geometry and show a
worker-side bbox candidate in the FDW Remote SQL.
EOF
