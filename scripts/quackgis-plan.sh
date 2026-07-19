#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-compose.sh"

explain() {
    role=$1
    sql=$2
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U "$role" \
        -d quackgis \
        -c "EXPLAIN (VERBOSE, COSTS OFF) $sql"
}

assert_bbox_plan() {
    label=$1
    plan=$2

    for fragment in '"minx" <=' '"maxx" >=' '"miny" <=' '"maxy" >='; do
        printf '%s\n' "$plan" | grep -F "$fragment" >/dev/null || {
            echo "$label is missing worker-side bbox fragment: $fragment" >&2
            exit 1
        }
    done
    printf '%s\n' "$plan" | grep -F 'Filter:' >/dev/null || {
        echo "$label is missing the local exact recheck" >&2
        exit 1
    }
}

qgis_plan=$(explain quackgis_reader \
    "SELECT id FROM public.features WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326)")
features_plan=$(explain quackgis_features \
    "SELECT ST_AsGeoJSON(geom), id, name::text FROM public.features WHERE ST_Intersects(geom, ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326)) LIMIT 10")
tile_plan=$(explain quackgis_tiles \
    "SELECT id FROM public.features WHERE geom && ST_Expand(ST_Transform(ST_TileEnvelope(8, 40, 87), 4326), (0.015625 * 360) / 2^8)")
limit_plan=$(explain quackgis_reader \
    "SELECT id FROM public.features WHERE id > 0 LIMIT 1")
extent_plan=$(explain quackgis_features \
    "SELECT ST_EstimatedExtent('public', 'features', 'geom')")
ogr_plan=$(explain quackgis_reader \
    "SELECT geom, id, name FROM public.features WHERE geom && ST_SetSRID('BOX3D(-123.2 49.1,-123 49.3)'::box3d, 4326)")
join_limit_plan=$(explain quackgis_reader \
    "SELECT f.id FROM public.features AS f JOIN (VALUES (1::bigint), (2::bigint)) AS v(id) ON f.id = v.id LIMIT 1")

printf '%s\n' '-- QGIS viewport --' "$qgis_plan"
printf '%s\n' '-- pg_featureserv bbox --' "$features_plan"
printf '%s\n' '-- Martin tile bbox --' "$tile_plan"
printf '%s\n' '-- scalar limit --' "$limit_plan"
printf '%s\n' '-- bounded feature metadata extent --' "$extent_plan"
printf '%s\n' '-- GDAL/OGR viewport --' "$ogr_plan"
printf '%s\n' '-- join limit stays local --' "$join_limit_plan"

assert_bbox_plan "QGIS viewport plan" "$qgis_plan"
assert_bbox_plan "pg_featureserv plan" "$features_plan"
assert_bbox_plan "Martin tile plan" "$tile_plan"
assert_bbox_plan "GDAL/OGR viewport plan" "$ogr_plan"
printf '%s\n' "$limit_plan" | grep -E 'Remote SQL: .* LIMIT 1$' >/dev/null || {
    echo "scalar LIMIT 1 was not pushed to the worker" >&2
    exit 1
}
if printf '%s\n' "$extent_plan" | grep -E 'Foreign Scan|Remote SQL' >/dev/null; then
    echo "pg_featureserv extent metadata unexpectedly scans the worker" >&2
    exit 1
fi
if printf '%s\n' "$join_limit_plan" | grep -E 'Remote SQL: .* LIMIT 1$' >/dev/null; then
    echo "join LIMIT 1 was unsafely pushed into a base scan" >&2
    exit 1
fi

result=$(
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT array_agg(id ORDER BY id) FROM public.features WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326)"
)

[ "$result" = "{1}" ] || {
    echo "unexpected viewport result: $result" >&2
    exit 1
}

echo "QuackGIS worker-side viewport, service bbox, and limit plans passed"
