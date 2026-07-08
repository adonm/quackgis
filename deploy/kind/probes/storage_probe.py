# SPDX-License-Identifier: Apache-2.0
import io
import sys

from probe_common import ORIGIN_WKB, ONE_WKB, pg_connect, quote_ident, table_name


def copy_bytea_hex(hex_wkb: str) -> str:
    return "\\\\x" + hex_wkb.lower()


def fetch_rows(cur, table: str):
    table_ref = f"public.{quote_ident(table)}"
    cur.execute(
        f"SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) "
        f"FROM {table_ref} ORDER BY id"
    )
    return cur.fetchall()


def main() -> int:
    table = table_name("storage_points")
    table_ref = f"public.{quote_ident(table)}"
    copy_data = "".join(
        [
            f"1\torigin\t{copy_bytea_hex(ORIGIN_WKB)}\n",
            f"2\tone\t{copy_bytea_hex(ONE_WKB)}\n",
        ]
    )

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(
                f"CREATE TABLE {table_ref} (id INT, name TEXT, geom BINARY)"
            )
            cur.copy_expert(
                f"COPY {table_ref} (id, name, geom) FROM STDIN",
                io.StringIO(copy_data),
            )
            before = fetch_rows(cur, table)
            cur.execute(f"CALL quackgis_compact_table('public.{table}')")
            after = fetch_rows(cur, table)
    finally:
        conn.close()

    expected = [(1, "origin", "POINT(0 0)"), (2, "one", "POINT(1 1)")]
    ok = before == after == expected
    print("storage_table", f"public.{table}")
    print("storage_rows", after)
    print("storage_ok", ok)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())
