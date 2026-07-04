#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""QuackGIS psycopg client test suite.

Tests client-level PostgreSQL compatibility through the psycopg (v3) driver:
  - Basic spatial query
  - Prepared statements / parameter binding
  - Result metadata (column names, types)
  - BI-tool metadata queries (pg_catalog, information_schema)
  - PostGIS compatibility introspection

Usage:
    python3 container/tests/test_psycopg.py

Environment:
    PG_HOST     default: 127.0.0.1
    PG_PORT     default: 55432
    PG_USER     default: postgres
    PG_PASSWORD default: quackgis
    PG_DB       default: postgres
"""

from __future__ import annotations

import os
import sys

try:
    import psycopg
    from psycopg.rows import dict_row
except ImportError:
    print("SKIP: psycopg not installed (pip install psycopg[binary])")
    sys.exit(0)

PG_HOST = os.environ.get("PG_HOST", "127.0.0.1")
PG_PORT = os.environ.get("PG_PORT", "55432")
PG_USER = os.environ.get("PG_USER", "postgres")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "quackgis")
PG_DB = os.environ.get("PG_DB", "postgres")

PASS = 0
FAIL = 0


def check(label: str, condition: bool, detail: str = "") -> None:
    global PASS, FAIL
    if condition:
        print(f"PASS {label}")
        PASS += 1
    else:
        print(f"FAIL {label} {detail}")
        FAIL += 1


def main() -> int:
    dsn = (
        f"host={PG_HOST} port={PG_PORT} user={PG_USER} "
        f"password={PG_PASSWORD} dbname={PG_DB}"
    )

    try:
        conn = psycopg.connect(dsn, row_factory=dict_row, connect_timeout=10)
    except Exception as e:
        print(f"FAIL connection {e}")
        return 1

    cur = conn.cursor()

    # ── 1. Basic spatial query ───────────────────────────────────────────

    try:
        cur.execute("SELECT st_astext(st_geomfromtext('POINT(1 2)')) AS result")
        row = cur.fetchone()
        check("basic spatial query", row["result"] == "POINT(1 2)", f"got {row}")
    except Exception as e:
        check("basic spatial query", False, str(e))

    # ── 2. Prepared statement / parameter binding ───────────────────────

    try:
        cur.execute(
            "SELECT st_astext(st_point(%s, %s)) AS result",
            (float(10), float(20)),
        )
        row = cur.fetchone()
        check(
            "prepared statement params",
            row["result"] == "POINT(10 20)",
            f"got {row}",
        )
    except Exception as e:
        check("prepared statement params", False, str(e))

    try:
        cur.execute(
            "SELECT st_area(st_geomfromtext(%s)) AS result",
            ("POLYGON((0 0,4 0,4 4,0 4,0 0))",),
        )
        row = cur.fetchone()
        check("prepared statement wkt", row["result"] == 16.0, f"got {row}")
    except Exception as e:
        check("prepared statement wkt", False, str(e))

    # ── 3. Result metadata ──────────────────────────────────────────────

    try:
        cur.execute(
            "SELECT st_point(1,2) AS geom, st_area(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) AS area"
        )
        desc = cur.description
        col_names = [d.name for d in desc]
        check(
            "result metadata columns",
            col_names == ["geom", "area"],
            f"got {col_names}",
        )
    except Exception as e:
        check("result metadata columns", False, str(e))

    # ── 4. BI tool metadata: pg_catalog ─────────────────────────────────

    try:
        cur.execute(
            "SELECT count(*) AS n FROM pg_catalog.pg_tables WHERE schemaname = 'public'"
        )
        row = cur.fetchone()
        check("pg_catalog pg_tables", row is not None and row["n"] >= 0)
    except Exception as e:
        check("pg_catalog pg_tables", False, str(e))

    try:
        cur.execute(
            "SELECT count(*) AS n FROM information_schema.tables WHERE table_schema = 'public'"
        )
        row = cur.fetchone()
        check("information_schema tables", row is not None)
    except Exception as e:
        check("information_schema tables", False, str(e))

    # ── 5. PostGIS introspection ────────────────────────────────────────

    try:
        cur.execute("SELECT postgis_version() AS version")
        row = cur.fetchone()
        check(
            "postgis_version",
            row is not None and "QUACKGIS" in str(row["version"]),
            f"got {row}",
        )
    except Exception as e:
        check("postgis_version", False, str(e))

    try:
        cur.execute("SELECT count(*) AS n FROM quackgis.compat_check()")
        row = cur.fetchone()
        check("quackgis.compat_check", row is not None and row["n"] > 0)
    except Exception as e:
        check("quackgis.compat_check", False, str(e))

    # ── 6. Transaction: BEGIN/COMMIT ────────────────────────────────────

    try:
        with conn.transaction():
            cur.execute("SELECT st_distance(st_point(0,0), st_point(3,4)) AS d")
            row = cur.fetchone()
            check("transaction commit", row["d"] == 5.0, f"got {row}")
    except Exception as e:
        check("transaction commit", False, str(e))

    # ── 7. Operators via SQL ────────────────────────────────────────────

    try:
        cur.execute(
            "SELECT (st_geomfromtext('POINT(0 0)'::text) "
            "       <-> st_geomfromtext('POINT(3 4)'::text)) AS d"
        )
        row = cur.fetchone()
        check("operator <-> via psycopg", row["d"] == 5.0, f"got {row}")
    except Exception as e:
        check("operator <-> via psycopg", False, str(e))

    # ── Summary ─────────────────────────────────────────────────────────

    conn.close()

    print(f"\npsycopg tests: PASS={PASS} FAIL={FAIL}")
    return 1 if FAIL > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
