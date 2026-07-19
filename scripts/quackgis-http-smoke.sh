#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
base_url=${QUACKGIS_HTTP_URL:-http://127.0.0.1:8080}

mkdir -p "$root/.tmp"
tmp=$(mktemp -d "$root/.tmp/quackgis-http.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

attempt=0
until curl -fsS "$base_url/features/collections" >"$tmp/collections.json"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        echo "QuackGIS HTTP edge did not become ready" >&2
        exit 1
    fi
    sleep 2
done

features_url="$base_url/features/collections/public.features/items?bbox=-123.2,49.1,-123.0,49.3&limit=10"
curl -fsS -D "$tmp/features.headers" "$features_url" >"$tmp/features.json"
curl -fsS -D "$tmp/feature.headers" \
    "$base_url/features/collections/public.features/items/1" >"$tmp/feature.json"
curl -fsS -D "$tmp/tilejson.headers" "$base_url/tiles/features" >"$tmp/tilejson.json"
tile_status=$(curl -sS -D "$tmp/tile.headers" -o "$tmp/tile.pbf" -w '%{http_code}' \
    "$base_url/tiles/features/8/40/87")
curl -fsS -D "$tmp/revision.headers" \
    "$base_url/tiles/revision-f3f65093582c" >"$tmp/revision.json"
revision_tile_status=$(curl -sS -D "$tmp/revision-tile.headers" \
    -o "$tmp/revision-tile.pbf" -w '%{http_code}' \
    "$base_url/tiles/revision-f3f65093582c/0/0/0")

python3 - \
    "$tmp/collections.json" \
    "$tmp/features.json" \
    "$tmp/feature.json" \
    "$tmp/tilejson.json" \
    "$tmp/revision.json" <<'PY'
import json
import sys

collections, features, feature_by_id, tilejson, revision = (
    json.load(open(path, encoding="utf-8")) for path in sys.argv[1:]
)

ids = [collection["id"] for collection in collections["collections"]]
assert ids == ["public.features"], ids
assert collections["collections"][0]["extent"]["spatial"]["bbox"] == [
    -123.1,
    48.9,
    -122.9,
    49.25,
], collections
assert features["type"] == "FeatureCollection", features
assert features["numberReturned"] == 1, features
feature = features["features"][0]
assert feature["id"] == "1", feature
assert feature["properties"] == {"id": 1, "name": "west"}, feature
assert feature["geometry"] == {
    "type": "Point",
    "coordinates": [-123.1, 49.2],
}, feature
assert feature_by_id == feature, feature_by_id
assert tilejson["tilejson"] == "3.0.0", tilejson
assert tilejson["vector_layers"] == [
    {"id": "features", "fields": {"id": "int8", "name": "text"}}
], tilejson
assert tilejson["bounds"] == [-123.1, 48.9, -122.9, 49.25], tilejson
assert tilejson["tiles"][0].endswith("/tiles/features/{z}/{x}/{y}"), tilejson
assert revision["name"] == "test_fixture_1.pmtiles", revision
assert revision["minzoom"] == 0 and revision["maxzoom"] == 0, revision
assert revision["tiles"][0].endswith(
    "/tiles/revision-f3f65093582c/{z}/{x}/{y}"
), revision
PY

for headers in features feature tilejson tile; do
    grep -i '^Cache-Control: no-store' "$tmp/$headers.headers" >/dev/null || {
        echo "$headers response is missing Cache-Control: no-store" >&2
        exit 1
    }
done

[ "$tile_status" = "200" ] || {
    echo "expected MVT status 200, got $tile_status" >&2
    exit 1
}
[ -s "$tmp/tile.pbf" ] || {
    echo "MVT response was empty" >&2
    exit 1
}
grep -i '^Content-Type: application/x-protobuf' "$tmp/tile.headers" >/dev/null || {
    echo "unexpected MVT content type" >&2
    exit 1
}

[ "$revision_tile_status" = "200" ] || {
    echo "expected PMTiles-backed MVT status 200, got $revision_tile_status" >&2
    exit 1
}
[ -s "$tmp/revision-tile.pbf" ] || {
    echo "PMTiles-backed MVT response was empty" >&2
    exit 1
}
for headers in revision revision-tile; do
    grep -i '^Cache-Control: public, max-age=31536000, immutable' \
        "$tmp/$headers.headers" >/dev/null || {
        echo "$headers response is missing immutable cache policy" >&2
        exit 1
    }
done

echo "QuackGIS OGC Features, dynamic MVT, and immutable PMTiles smoke passed"
