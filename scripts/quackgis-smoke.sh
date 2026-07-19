#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-compose.sh"

attempt=0
until "$compose" exec -T postgres psql \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "SELECT id, name, GeometryType(geom), ST_SRID(geom), ST_AsText(geom) AS geom FROM public.features ORDER BY id"; do
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
        -c "SELECT count(*) FROM public.features"
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

geometry_contract=$(
    "$compose" exec -T postgres psql \
        -At \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT format_type(a.atttypid, a.atttypmod) FROM pg_attribute a WHERE a.attrelid = 'remote.features_export'::regclass AND a.attname = 'geom'"
)

[ "$geometry_contract" = "geometry(Point,4326)" ] || {
    echo "unexpected geometry contract: $geometry_contract" >&2
    exit 1
}

extent=$(
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT ST_Extent(geom)::text FROM public.features"
)

[ "$extent" = "BOX(-123.1 48.9,-122.9 49.25)" ] || {
    echo "unexpected remote extent: $extent" >&2
    exit 1
}

geometry_cases=$(
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT string_agg(id || ':' || coalesce(GeometryType(geom) || ':' || ST_SRID(geom), 'NULL'), ',' ORDER BY id) FROM remote.geometry_contract_export"
)

[ "$geometry_cases" = "1:POINT:4326,2:NULL" ] || {
    echo "unexpected geometry edge cases: $geometry_cases" >&2
    exit 1
}

if "$compose" exec -T postgres psql \
    -XAt \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "SELECT geom FROM remote.geometry_malformed_export" >/dev/null 2>&1; then
    echo "malformed WKB unexpectedly crossed the FDW boundary" >&2
    exit 1
fi

if "$compose" exec -T postgres psql \
    -XAt \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "SELECT geom FROM remote.geometry_wrong_family_export" >/dev/null 2>&1; then
    echo "wrong-family geometry unexpectedly crossed the FDW boundary" >&2
    exit 1
fi

mkdir -p "$root/.tmp"
concurrent_tmp=$(mktemp -d "$root/.tmp/quackgis-readers.XXXXXX")
trap 'rm -rf "$concurrent_tmp"' EXIT INT TERM

concurrent_read() {
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT count(*) FROM public.features"
}

concurrent_read >"$concurrent_tmp/first" &
first_pid=$!
concurrent_read >"$concurrent_tmp/second" &
second_pid=$!
wait "$first_pid"
wait "$second_pid"

for result_file in "$concurrent_tmp/first" "$concurrent_tmp/second"; do
    [ "$(cat "$result_file")" = "3" ] || {
        echo "concurrent remote reader returned an unexpected result" >&2
        exit 1
    }
done

rm -rf "$concurrent_tmp"
trap - EXIT INT TERM

echo "QuackGIS native geometry, edge cases, extent, and read-only smoke passed"
