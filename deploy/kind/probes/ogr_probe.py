# SPDX-License-Identifier: Apache-2.0
import os
import sys

from probe_common import (
    load_geojson,
    pg_connect,
    pg_dsn,
    quote_ident,
    require_equal,
    run_cmd,
    seed_point_table,
    table_name,
    write_geojson,
)


def main() -> int:
    read_table = table_name("ogr_probe")
    load_table = table_name("ogr_load")
    keyless_table = table_name("ogr_keyless")

    conn = pg_connect()
    conn.autocommit = True
    try:
        seed_point_table(conn, read_table, geom_col="wkb_geometry")
        with conn.cursor() as cur:
            cur.execute(f"CREATE TABLE public.{quote_ident(load_table)} (id INT, wkb_geometry BINARY, name TEXT)")
            cur.execute(
                f"CREATE TABLE public.{quote_ident(keyless_table)} (wkb_geometry BINARY, name TEXT)"
            )
            cur.execute(
                f"INSERT INTO public.{quote_ident(keyless_table)} (wkb_geometry, name) VALUES "
                "(X'010100000000000000000000000000000000000000', 'keyless-origin'), "
                "(X'0101000000000000000000F03F000000000000F03F', 'keyless-one') "
                "RETURNING \"_quackgis_rowid\", name"
            )
            print("ogr_keyless_seed_rows", cur.fetchall())
    finally:
        conn.close()

    dsn = pg_dsn()

    # QuackGIS serves WKB-backed layers over the PostgreSQL wire, but it does
    # not need GDAL's PostGIS-extension datasource-open probes for this target.
    # Disabling those probes keeps the maintained OGR gate focused on the
    # portable PostgreSQL-driver read/load/readback contract.
    os.environ["PG_USE_POSTGIS"] = "NO"
    run_cmd(["ogrinfo", dsn, read_table, "-so"])
    run_cmd(["ogr2ogr", "-f", "GeoJSON", "/tmp/out.geojson", dsn, read_table])

    fc = load_geojson("/tmp/out.geojson")
    features = fc["features"]
    names = sorted(feature["properties"].get("name") for feature in features)
    geometry_types = [feature["geometry"]["type"] for feature in features]
    print("feature_count", len(features))
    print("names", names)
    print("geometry_types", geometry_types)
    require_equal(len(features), 2, "feature_count")
    require_equal(names, ["one", "origin"], "names")
    require_equal(geometry_types, ["Point", "Point"], "geometry_types")

    write_geojson(
        "/tmp/load.geojson",
        {
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": {"name": "load-a", "category": "client"},
                    "geometry": {"type": "Point", "coordinates": [2.0, 2.0]},
                },
                {
                    "type": "Feature",
                    "properties": {"name": "load-b", "category": "client"},
                    "geometry": {"type": "Point", "coordinates": [3.0, 3.0]},
                },
            ],
        },
    )

    os.environ["PG_USE_COPY"] = "NO"
    run_cmd(
        [
            "ogr2ogr",
            "-f",
            "PostgreSQL",
            dsn,
            "/tmp/load.geojson",
            "-append",
            "-addfields",
            "-nln",
            load_table,
            "-nlt",
            "POINT",
            "-lco",
            "GEOMETRY_NAME=wkb_geometry",
        ]
    )
    run_cmd(["ogr2ogr", "-f", "GeoJSON", "/tmp/load_out.geojson", dsn, load_table])

    rows_conn = pg_connect()
    try:
        with rows_conn.cursor() as cur:
            cur.execute(
                f"SELECT name, category, ST_AsText(ST_GeomFromWKB(wkb_geometry)) "
                f"FROM public.{quote_ident(load_table)} ORDER BY name"
            )
            rows = cur.fetchall()
    finally:
        rows_conn.close()
    print("loaded_rows", rows)
    require_equal(
        rows,
        [("load-a", "client", "POINT(2 2)"), ("load-b", "client", "POINT(3 3)")],
        "loaded_rows",
    )

    fc = load_geojson("/tmp/load_out.geojson")
    features = fc["features"]
    names = sorted(feature["properties"].get("name") for feature in features)
    categories = sorted(feature["properties"].get("category") for feature in features)
    geometry_types = [feature["geometry"]["type"] for feature in features]
    print("load_feature_count", len(features))
    print("load_names", names)
    print("load_categories", categories)
    print("load_geometry_types", geometry_types)
    require_equal(len(features), 2, "load_feature_count")
    require_equal(names, ["load-a", "load-b"], "load_names")
    require_equal(categories, ["client", "client"], "load_categories")
    require_equal(geometry_types, ["Point", "Point"], "load_geometry_types")

    run_cmd(["ogrinfo", dsn, keyless_table, "-so"])
    run_cmd(["ogr2ogr", "-f", "GeoJSON", "/tmp/keyless.geojson", dsn, keyless_table])
    keyless_fc = load_geojson("/tmp/keyless.geojson")
    keyless_features = keyless_fc["features"]
    keyless_names = sorted(feature["properties"].get("name") for feature in keyless_features)
    keyless_rowids = sorted(rowid_from_geojson(feature) for feature in keyless_features)
    print("ogr_keyless_feature_count", len(keyless_features))
    print("ogr_keyless_names", keyless_names)
    print("ogr_keyless_rowids", keyless_rowids)
    require_equal(len(keyless_features), 2, "ogr_keyless_feature_count")
    require_equal(keyless_names, ["keyless-one", "keyless-origin"], "ogr_keyless_names")
    require_equal(keyless_rowids, ["1", "2"], "ogr_keyless_rowids")

    compact_conn = pg_connect()
    compact_conn.autocommit = True
    try:
        with compact_conn.cursor() as cur:
            cur.execute(
                f"SELECT \"_quackgis_rowid\", name FROM public.{quote_ident(keyless_table)} "
                "ORDER BY \"_quackgis_rowid\""
            )
            before_compact = cur.fetchall()
            cur.execute(f"CALL quackgis_compact_table('public.{keyless_table}')")
            cur.execute(
                f"SELECT \"_quackgis_rowid\", name FROM public.{quote_ident(keyless_table)} "
                "ORDER BY \"_quackgis_rowid\""
            )
            after_compact = cur.fetchall()
    finally:
        compact_conn.close()
    print("ogr_keyless_before_compact", before_compact)
    print("ogr_keyless_after_compact", after_compact)
    require_equal(after_compact, before_compact, "ogr_keyless_compact_rows")

    run_cmd(["ogr2ogr", "-f", "GeoJSON", "/tmp/keyless_after_compact.geojson", dsn, keyless_table])
    compact_fc = load_geojson("/tmp/keyless_after_compact.geojson")
    compact_rowids = sorted(rowid_from_geojson(feature) for feature in compact_fc["features"])
    print("ogr_keyless_compact_rowids", compact_rowids)
    require_equal(compact_rowids, ["1", "2"], "ogr_keyless_compact_rowids")
    print("ogr_keyless_compact_ok True")
    return 0


def rowid_from_geojson(feature) -> str:
    raw = feature.get("properties", {}).get("_quackgis_rowid")
    if raw is None:
        raw = feature.get("id")
    value = str(raw)
    if "." in value:
        value = value.rsplit(".", 1)[-1]
    return value


if __name__ == "__main__":
    sys.exit(main())
