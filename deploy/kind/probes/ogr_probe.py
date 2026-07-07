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

    conn = pg_connect()
    conn.autocommit = True
    try:
        seed_point_table(conn, read_table, geom_col="wkb_geometry")
        with conn.cursor() as cur:
            cur.execute(f"CREATE TABLE public.{quote_ident(load_table)} (id INT, wkb_geometry BINARY, name TEXT)")
    finally:
        conn.close()

    dsn = pg_dsn()
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
