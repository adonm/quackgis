# SPDX-License-Identifier: Apache-2.0
import sys

from probe_common import ONE_WKB, ORIGIN_WKB, pg_connect, quote_ident, seed_point_table, table_name


def main() -> int:
    table = table_name("geoserver_probe")
    keyless_table = table_name("geoserver_keyless")
    conn = pg_connect()
    conn.autocommit = True
    try:
        seed_point_table(conn, table)
        with conn.cursor() as cur:
            cur.execute(f"CREATE TABLE public.{quote_ident(keyless_table)} (name TEXT, geom BINARY)")
            cur.execute(
                f"INSERT INTO public.{quote_ident(keyless_table)} (name, geom) VALUES "
                f"('keyless-origin', X'{ORIGIN_WKB}'), "
                f"('keyless-one', X'{ONE_WKB}') "
                "RETURNING \"_quackgis_rowid\", name"
            )
            print("geoserver_keyless_seed_rows", cur.fetchall())
    finally:
        conn.close()
    print("geoserver_seed_table", table)
    print("geoserver_keyless_seed_table", keyless_table)
    return 0


if __name__ == "__main__":
    sys.exit(main())
