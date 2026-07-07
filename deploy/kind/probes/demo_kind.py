# SPDX-License-Identifier: Apache-2.0
import struct
import sys

from probe_common import (
    ONE_WKB,
    ORIGIN_WKB,
    pg_connect,
    pg_dsn,
    quackgis_host,
    quackgis_port,
    quote_ident,
)


def polygon_wkb_hex(coords: list[tuple[float, float]]) -> str:
    if coords[0] != coords[-1]:
        coords = [*coords, coords[0]]
    payload = bytearray()
    payload.extend(struct.pack("<BI", 1, 3))  # little endian, Polygon
    payload.extend(struct.pack("<I", 1))
    payload.extend(struct.pack("<I", len(coords)))
    for x, y in coords:
        payload.extend(struct.pack("<dd", x, y))
    return payload.hex()


def reset_table(cur, table: str):
    table_ref = f"public.{quote_ident(table)}"
    try:
        cur.execute(f"DELETE FROM {table_ref}")
    except Exception:
        cur.connection.rollback()
        cur.execute(f"CREATE TABLE {table_ref} (id INT, geom BINARY, name TEXT)")


def main() -> int:
    points_table = "demo_points"
    polygons_table = "demo_polygons"
    host = quackgis_host()
    port = quackgis_port()
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            reset_table(cur, points_table)
            cur.execute(
                f"INSERT INTO public.{quote_ident(points_table)} VALUES "
                f"(1, X'{ORIGIN_WKB}', 'origin'), "
                f"(2, X'{ONE_WKB}', 'one')"
            )
            reset_table(cur, polygons_table)
            square = polygon_wkb_hex([(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)])
            triangle = polygon_wkb_hex([(3.0, 0.0), (5.0, 0.0), (4.0, 2.0)])
            cur.execute(
                f"INSERT INTO public.{quote_ident(polygons_table)} VALUES "
                f"(1, X'{square}', 'square'), "
                f"(2, X'{triangle}', 'triangle')"
            )
            cur.execute(
                f"SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) "
                f"FROM public.{quote_ident(points_table)} ORDER BY id"
            )
            point_rows = cur.fetchall()
            cur.execute(
                f"SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) "
                f"FROM public.{quote_ident(polygons_table)} ORDER BY id"
            )
            polygon_rows = cur.fetchall()
    finally:
        conn.close()

    ok = (
        point_rows == [(1, "origin", "POINT(0 0)"), (2, "one", "POINT(1 1)")]
        and len(polygon_rows) == 2
    )
    print("demo_tables", [f"public.{points_table}", f"public.{polygons_table}"])
    print("demo_points", point_rows)
    print("demo_polygons", polygon_rows)
    print(
        "qgis_connection",
        f"host={host} port={port} dbname=quackgis user=postgres "
        f"tables=public.{points_table},public.{polygons_table}",
    )
    print("ogr_points", f"ogrinfo '{pg_dsn()}' {points_table} -so")
    print("ogr_polygons", f"ogrinfo '{pg_dsn()}' {polygons_table} -so")
    print(
        "sample_sql",
        f"SELECT name, ST_AsText(ST_GeomFromWKB(geom)) FROM public.{quote_ident(points_table)} ORDER BY id;",
    )
    print("demo_ok", ok)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())
