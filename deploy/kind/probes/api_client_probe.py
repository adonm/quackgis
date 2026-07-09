# SPDX-License-Identifier: Apache-2.0
"""Kind API/client compatibility surface probe.

This is the containerized counterpart to `just api-client-local-smoke`: it runs
inside the maintained Kind network and proves the protocol/catalog surfaces that
Python/API/BI clients need before each named client gets its own heavier probe.
"""

from __future__ import annotations

import sys

import psycopg2

from probe_common import (
    ORIGIN_WKB,
    ONE_WKB,
    pg_connect,
    quote_ident,
    require,
    require_equal,
    table_name,
)


def point_wkb(x: float, y: float) -> bytes:
    import struct

    return b"\x01" + struct.pack("<I", 1) + struct.pack("<dd", x, y)


def seed(conn, table: str) -> None:
    table_ref = f"public.{quote_ident(table)}"
    with conn.cursor() as cur:
        cur.execute(
            f"CREATE TABLE {table_ref} "
            "(id INT, geom BINARY, name TEXT, category TEXT)"
        )
        cur.execute(
            f"INSERT INTO {table_ref} (id, geom, name, category) VALUES "
            f"(1, X'{ORIGIN_WKB}', 'origin', 'alpha'), "
            f"(2, X'{ONE_WKB}', 'one', 'alpha'), "
            "(3, X'010100000000000000000014400000000000001440', 'far', 'beta')"
        )


def assert_psycopg_surface(conn, table: str) -> None:
    with conn.cursor() as cur:
        cur.execute("SELECT ST_AsText(ST_GeomFromText(%s))", ("POINT(3 4)",))
        require_equal(cur.fetchone()[0], "POINT(3 4)", "text WKT parameter")

        cur.execute(
            "SELECT ST_AsText(ST_GeomFromWKB(%s))",
            (psycopg2.Binary(point_wkb(3.0, 4.0)),),
        )
        require_equal(cur.fetchone()[0], "POINT(3 4)", "binary WKB parameter")

        cur.execute(
            f"SELECT ST_AsEWKB(geom) FROM public.{quote_ident(table)} WHERE id = 1"
        )
        ewkb = cur.fetchone()[0]
        require(bytes(ewkb) == bytes.fromhex(ORIGIN_WKB), "ST_AsEWKB bytes changed")
    print("api_client_psycopg_surface text_param=True binary_wkb=True ewkb=True")


def assert_reflection_surface(conn, table: str) -> int:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT COUNT(*) FROM information_schema.tables "
            "WHERE table_schema = 'public' AND table_name = %s",
            (table,),
        )
        require_equal(cur.fetchone()[0], 1, "information_schema table count")
        cur.execute(
            "SELECT column_name FROM information_schema.columns "
            "WHERE table_schema = 'public' AND table_name = %s "
            "ORDER BY ordinal_position",
            (table,),
        )
        columns = [row[0] for row in cur.fetchall()]
    for expected in ("id", "geom", "name", "category"):
        require(expected in columns, f"missing reflected column {expected}: {columns!r}")
    require(
        all(not column.startswith("_qg_") for column in columns),
        f"hidden layout columns leaked through public reflection: {columns!r}",
    )
    print(f"api_client_sqlalchemy_surface columns={columns!r}")
    return len(columns)


def assert_geopandas_surface(conn, table: str) -> int:
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT id, name, ST_AsEWKB(geom) AS geom "
            f"FROM public.{quote_ident(table)} ORDER BY id"
        )
        rows = cur.fetchall()
    require_equal(len(rows), 3, "GeoPandas-style feature count")
    require(bytes(rows[0][2]) == bytes.fromhex(ORIGIN_WKB), "first WKB changed")
    print("api_client_geopandas_surface feature_count=3 wkb=True crs_documented=srid0")
    return len(rows)


def assert_pgfeatureserv_surface(conn, table: str) -> int:
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT id, name FROM public.{quote_ident(table)} "
            "WHERE ST_Intersects("
            "ST_GeomFromWKB(geom), "
            "ST_GeomFromWKB(ST_MakeEnvelope(-0.5, -0.5, 0.5, 0.5, 3857))"
            ") ORDER BY id"
        )
        rows = cur.fetchall()
    require_equal(rows, [(1, "origin")], "pg_featureserv bbox rows")
    print("api_client_pgfeatureserv_surface bbox_count=1 properties=True")
    return len(rows)


def assert_bi_surface(conn, table: str) -> int:
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT category, COUNT(*) FROM public.{quote_ident(table)} "
            "GROUP BY category ORDER BY category"
        )
        rows = cur.fetchall()
    require_equal(rows, [("alpha", 2), ("beta", 1)], "BI grouped aggregate")
    print(f"api_client_bi_surface groups={len(rows)} grouped={rows!r}")
    return len(rows)


def assert_mvt_surface(conn, table: str) -> int:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT ST_AsMVT("
            "ST_AsMVTGeom(geom, ST_MakeEnvelope(-1.0, -1.0, 2.0, 2.0, 3857), 4096, 64, true)"
            f") FROM public.{quote_ident(table)} WHERE id <= 2"
        )
        tile = cur.fetchone()[0]
    tile_bytes = len(bytes(tile))
    require(tile_bytes > 0, "MVT tile was empty")
    print(f"api_client_mvt_surface tile_bytes={tile_bytes}")
    return tile_bytes


def main() -> int:
    table = table_name("api_client_probe")
    conn = pg_connect()
    conn.autocommit = True
    try:
        seed(conn, table)
        assert_psycopg_surface(conn, table)
        reflected_columns = assert_reflection_surface(conn, table)
        feature_count = assert_geopandas_surface(conn, table)
        bbox_count = assert_pgfeatureserv_surface(conn, table)
        groups = assert_bi_surface(conn, table)
        tile_bytes = assert_mvt_surface(conn, table)
    finally:
        conn.close()

    print(
        "api_client_summary "
        f"feature_count={feature_count} "
        f"reflected_columns={reflected_columns} "
        f"bbox_count={bbox_count} "
        f"groups={groups} "
        f"tile_bytes={tile_bytes}"
    )
    print("api_client_probe_ok True")
    return 0


if __name__ == "__main__":
    sys.exit(main())
