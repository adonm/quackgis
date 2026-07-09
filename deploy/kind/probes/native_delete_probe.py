# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import os
import struct
import sys

import psycopg2

from probe_common import pg_connect, quote_ident, require, require_equal, table_name


def point_wkb_hex(x: float, y: float) -> str:
    return struct.pack("<BIdd", 1, 1, x, y).hex()


def table_ref(table: str) -> str:
    return f"public.{quote_ident(table)}"


def ducklake_table_ref(table: str) -> str:
    return f"quackgis.main.{quote_ident(table)}"


def fetch_rows(cur, table: str):
    cur.execute(
        f"SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) "
        f"FROM {table_ref(table)} ORDER BY id"
    )
    return cur.fetchall()


def insert_pair(cur, table: str, rows: list[tuple[int, str, str]]) -> None:
    values = ", ".join(
        f"({row_id}, X'{wkb}', '{name}')" for row_id, name, wkb in rows
    )
    cur.execute(f"INSERT INTO {table_ref(table)} (id, geom, name) VALUES {values}")


def metadata_counts(table: str) -> tuple[int, int, int, int, int]:
    catalog_url = os.environ.get("QUACKGIS_CATALOG_URL")
    catalog_name = os.environ.get("QUACKGIS_DUCKLAKE_CATALOG_NAME")
    require(catalog_url is not None, "missing QUACKGIS_CATALOG_URL for metadata check")
    require(
        catalog_name is not None,
        "missing QUACKGIS_DUCKLAKE_CATALOG_NAME for metadata check",
    )
    require(
        catalog_url.startswith(("postgres://", "postgresql://")),
        f"native DML metadata check requires PostgreSQL catalog URL, got {catalog_url!r}",
    )

    with psycopg2.connect(catalog_url) as conn, conn.cursor() as cur:
        cur.execute(
            """
            WITH target_table AS (
                SELECT t.table_id
                FROM ducklake_table t
                JOIN ducklake_schema s ON s.schema_id = t.schema_id
                JOIN ducklake_catalog_schema_map csm ON csm.schema_id = s.schema_id
                JOIN ducklake_catalog c ON c.catalog_id = csm.catalog_id
                WHERE c.catalog_name = %s
                  AND s.schema_name = 'main'
                  AND t.table_name = %s
            ), delete_snapshots AS (
                SELECT DISTINCT df.begin_snapshot AS snapshot_id
                FROM ducklake_delete_file df
                JOIN target_table tt ON tt.table_id = df.table_id
            )
            SELECT
                (SELECT COUNT(*)::INT
                 FROM ducklake_delete_file df
                 JOIN target_table tt ON tt.table_id = df.table_id) AS delete_files,
                (SELECT COUNT(DISTINCT df.data_file_id)::INT
                 FROM ducklake_delete_file df
                 JOIN target_table tt ON tt.table_id = df.table_id) AS affected_data_files,
                (SELECT COUNT(*)::INT FROM delete_snapshots) AS delete_snapshots,
                (SELECT COUNT(*)::INT
                 FROM ducklake_data_file data
                 JOIN target_table tt ON tt.table_id = data.table_id
                 WHERE data.begin_snapshot IN (SELECT snapshot_id FROM delete_snapshots)) AS appended_files,
                (SELECT COUNT(*)::INT
                 FROM ducklake_data_file data
                 JOIN target_table tt ON tt.table_id = data.table_id
                 WHERE data.end_snapshot IN (SELECT snapshot_id FROM delete_snapshots)) AS retired_files
            """,
            (catalog_name, table),
        )
        row = cur.fetchone()
    require(row is not None, "missing native DML metadata result")
    return row[0], row[1], row[2], row[3], row[4]


def main() -> int:
    table = table_name("native_delete_points")
    update_table = table_name("native_update_points")
    compact_table = table_name("native_compact_points")
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"CREATE TABLE {table_ref(table)} (id INT, geom BINARY, name TEXT)")
            insert_pair(
                cur,
                table,
                [
                    (1, "a", point_wkb_hex(0.0, 0.0)),
                    (2, "b", point_wkb_hex(1.0, 1.0)),
                ],
            )
            insert_pair(
                cur,
                table,
                [
                    (3, "c", point_wkb_hex(2.0, 2.0)),
                    (4, "d", point_wkb_hex(3.0, 3.0)),
                ],
            )
            cur.execute(f"DELETE FROM {table_ref(table)} WHERE id = 2 OR id = 4")
            deleted = cur.rowcount
            rows = fetch_rows(cur, table)

            cur.execute(
                f"CREATE TABLE {table_ref(update_table)} (id INT, geom BINARY, name TEXT)"
            )
            insert_pair(
                cur,
                update_table,
                [
                    (1, "a", point_wkb_hex(0.0, 0.0)),
                    (2, "b", point_wkb_hex(1.0, 1.0)),
                ],
            )
            insert_pair(
                cur,
                update_table,
                [
                    (3, "c", point_wkb_hex(2.0, 2.0)),
                    (4, "d", point_wkb_hex(3.0, 3.0)),
                ],
            )
            cur.execute(
                f"UPDATE {table_ref(update_table)} SET name = 'updated' "
                "WHERE id = 2 OR id = 4"
            )
            updated = cur.rowcount
            update_rows = fetch_rows(cur, update_table)

            cur.execute(
                f"CREATE TABLE {table_ref(compact_table)} "
                "(id INT, captured_minute INT, geom BINARY, name TEXT)"
            )
            cur.execute(
                f"INSERT INTO {table_ref(compact_table)} "
                "(id, captured_minute, geom, name) VALUES "
                f"(1, 10, X'{point_wkb_hex(0.0, 0.0)}', 'a')"
            )
            cur.execute(
                f"INSERT INTO {table_ref(compact_table)} "
                "(id, captured_minute, geom, name) VALUES "
                f"(2, 80, X'{point_wkb_hex(2.0, 2.0)}', 'b')"
            )
            cur.execute(
                f"INSERT INTO {table_ref(compact_table)} "
                "(id, captured_minute, geom, name) VALUES "
                f"(3, 10, X'{point_wkb_hex(4.0, 4.0)}', 'c')"
            )
            cur.execute(
                f"SELECT _qg_time_bucket, _qg_space_bucket "
                f"FROM {ducklake_table_ref(compact_table)} WHERE id = 1"
            )
            bucket = cur.fetchone()
            require(bucket is not None, "missing native compaction bucket")
            time_bucket, space_bucket = bucket
            cur.execute(
                f"CALL quackgis_compact_table('public.{compact_table}', "
                f"{time_bucket}, {space_bucket})"
            )
            compact_rows = fetch_rows(cur, compact_table)
    finally:
        conn.close()

    require_equal(deleted, 2, "native DELETE row count")
    require_equal(rows, [(1, "a", "POINT(0 0)"), (3, "c", "POINT(2 2)")], "survivors")

    delete_files, affected_data_files, delete_snapshots, appended_files, retired_files = metadata_counts(table)
    require_equal(delete_files, 2, "native DELETE metadata delete files")
    require_equal(affected_data_files, 2, "native DELETE metadata affected data files")
    require_equal(delete_snapshots, 1, "native DELETE metadata snapshot count")
    require_equal(appended_files, 0, "native DELETE should not append data files")
    require_equal(retired_files, 0, "native DELETE should not retire data files")

    require_equal(updated, 2, "native UPDATE row count")
    require_equal(
        update_rows,
        [
            (1, "a", "POINT(0 0)"),
            (2, "updated", "POINT(1 1)"),
            (3, "c", "POINT(2 2)"),
            (4, "updated", "POINT(3 3)"),
        ],
        "updated rows",
    )
    (
        update_delete_files,
        update_affected_data_files,
        update_delete_snapshots,
        update_appended_files,
        update_retired_files,
    ) = metadata_counts(update_table)
    require_equal(update_delete_files, 2, "native UPDATE metadata delete files")
    require_equal(
        update_affected_data_files, 2, "native UPDATE metadata affected data files"
    )
    require_equal(update_delete_snapshots, 1, "native UPDATE metadata snapshot count")
    require_equal(update_appended_files, 1, "native UPDATE appended data files")
    require_equal(update_retired_files, 0, "native UPDATE retired data files")

    require_equal(
        compact_rows,
        [
            (1, "a", "POINT(0 0)"),
            (2, "b", "POINT(2 2)"),
            (3, "c", "POINT(4 4)"),
        ],
        "compacted rows",
    )
    (
        compact_delete_files,
        compact_affected_data_files,
        compact_delete_snapshots,
        compact_appended_files,
        compact_retired_files,
    ) = metadata_counts(compact_table)
    require_equal(compact_delete_files, 2, "native COMPACT metadata delete files")
    require_equal(
        compact_affected_data_files, 2, "native COMPACT metadata affected data files"
    )
    require_equal(compact_delete_snapshots, 1, "native COMPACT metadata snapshot count")
    require_equal(compact_appended_files, 1, "native COMPACT appended data files")
    require_equal(compact_retired_files, 0, "native COMPACT retired data files")

    print(
        "native_delete",
        f"table=public.{table}",
        f"deleted={deleted}",
        f"delete_files={delete_files}",
        f"affected_data_files={affected_data_files}",
        f"delete_snapshots={delete_snapshots}",
    )
    print("native_delete_rows", rows)
    print("native_delete_ok", True)
    print(
        "native_update",
        f"table=public.{update_table}",
        f"updated={updated}",
        f"delete_files={update_delete_files}",
        f"affected_data_files={update_affected_data_files}",
        f"delete_snapshots={update_delete_snapshots}",
        f"appended_files={update_appended_files}",
    )
    print("native_update_rows", update_rows)
    print("native_update_ok", True)
    print(
        "native_compact",
        f"table=public.{compact_table}",
        f"delete_files={compact_delete_files}",
        f"affected_data_files={compact_affected_data_files}",
        f"delete_snapshots={compact_delete_snapshots}",
        f"appended_files={compact_appended_files}",
        f"retired_files={compact_retired_files}",
    )
    print("native_compact_rows", compact_rows)
    print("native_compact_ok", True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
