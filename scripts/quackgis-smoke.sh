#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-compose.sh"

attempt=0
until "$compose" exec -T postgres psql \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "SELECT id, name, ST_AsText(geom) AS geom FROM public.features_unbounded ORDER BY id"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        echo "QuackGIS stack did not become queryable" >&2
        exit 1
    fi
    sleep 2
done

rows=$(
    "$compose" exec -T postgres psql \
        -At \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT count(*) FROM public.features_unbounded"
)

[ "$rows" = "3" ] || {
    echo "expected 3 remote rows, got $rows" >&2
    exit 1
}

if "$compose" exec -T postgres psql \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "DELETE FROM remote.features_export" >/dev/null 2>&1; then
    echo "read-only role unexpectedly deleted a remote row" >&2
    exit 1
fi

echo "QuackGIS scalar/geometry wiring smoke passed"
