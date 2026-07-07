# SPDX-License-Identifier: Apache-2.0
import sys

from probe_common import pg_connect, seed_point_table, table_name


def main() -> int:
    table = table_name("geoserver_probe")
    conn = pg_connect()
    conn.autocommit = True
    try:
        seed_point_table(conn, table)
    finally:
        conn.close()
    print("geoserver_seed_table", table)
    return 0


if __name__ == "__main__":
    sys.exit(main())
