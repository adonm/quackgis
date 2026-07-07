# SPDX-License-Identifier: Apache-2.0
"""Shared helpers for Kind client compatibility probes.

The probe boundary is QuackGIS's PostgreSQL wire service. Keep client-specific
assertions in the individual scripts and put deterministic table naming,
connection setup, seed fixtures, command execution, and common assertions here.
"""

from __future__ import annotations

import json
import os
import random
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable, Sequence

import psycopg2

ORIGIN_WKB = "010100000000000000000000000000000000000000"
ONE_WKB = "0101000000000000000000F03F000000000000F03F"

OGR_STDERR_NOISE = (
    'unexpected field count in "D" message',
    'ERROR 1: ',
)


def env(name: str, default: str | None = None) -> str:
    value = os.environ.get(name, default)
    if value is None:
        raise RuntimeError(f"missing required environment variable {name}")
    return value


def quackgis_host() -> str:
    return env("QUACKGIS_HOST", "quackgis.quackgis.svc.cluster.local")


def quackgis_port() -> int:
    return int(env("QUACKGIS_PORT", "5434"))


def pg_connect(*, host: str | None = None, port: int | None = None, dbname: str = "quackgis"):
    return psycopg2.connect(
        host=host or quackgis_host(),
        port=port or quackgis_port(),
        user="postgres",
        dbname=dbname,
    )


def pg_dsn(*, host: str | None = None, port: int | None = None, dbname: str = "quackgis") -> str:
    return f"PG:host={host or quackgis_host()} port={port or quackgis_port()} user=postgres dbname={dbname}"


def quote_ident(identifier: str) -> str:
    return '"' + identifier.replace('"', '""') + '"'


def table_ref(schema: str, table: str) -> str:
    return f"{quote_ident(schema)}.{quote_ident(table)}"


def table_name(prefix: str, env_name: str | None = None) -> str:
    if env_name and os.environ.get(env_name):
        return os.environ[env_name]
    if os.environ.get("POD_UID"):
        suffix = re.sub(r"[^A-Za-z0-9_]", "_", os.environ["POD_UID"])
    else:
        suffix = f"{os.getpid()}_{random.randint(1, 1_000_000_000)}"
    return f"{prefix}_{suffix}"


def create_point_table(
    conn,
    table: str,
    *,
    schema: str = "public",
    geom_col: str = "geom",
    id_col: str = "id",
):
    with conn.cursor() as cur:
        cur.execute(
            f"CREATE TABLE {table_ref(schema, table)} "
            f"({quote_ident(id_col)} INT, {quote_ident(geom_col)} BINARY, name TEXT)"
        )


def seed_two_points(
    conn,
    table: str,
    *,
    schema: str = "public",
    geom_col: str = "geom",
    id_col: str = "id",
):
    with conn.cursor() as cur:
        cur.execute(
            f"INSERT INTO {table_ref(schema, table)} "
            f"({quote_ident(id_col)}, {quote_ident(geom_col)}, name) VALUES "
            f"(1, X'{ORIGIN_WKB}', 'origin'), "
            f"(2, X'{ONE_WKB}', 'one')"
        )


def seed_point_table(conn, table: str, *, schema: str = "public", geom_col: str = "geom"):
    create_point_table(conn, table, schema=schema, geom_col=geom_col)
    seed_two_points(conn, table, schema=schema, geom_col=geom_col)


def query_all(sql: str, params: Sequence[Any] | None = None):
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            cur.execute(sql, params or ())
            return cur.fetchall()
    finally:
        conn.close()


def require(condition: bool, message: str):
    if not condition:
        raise RuntimeError(message)


def require_equal(actual: Any, expected: Any, label: str):
    if actual != expected:
        raise RuntimeError(f"unexpected {label}: {actual!r} != {expected!r}")


def load_geojson(path: str | Path) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def write_geojson(path: str | Path, feature_collection: dict[str, Any]):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(feature_collection, handle)


def run_cmd(args: Sequence[str], *, filter_stderr: Iterable[str] = OGR_STDERR_NOISE):
    proc = subprocess.run(args, check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if proc.stdout:
        print(proc.stdout, end="")

    filters = tuple(filter_stderr)
    stderr_lines = []
    for line in proc.stderr.splitlines():
        if not line.strip():
            continue
        if any(pattern in line for pattern in filters):
            continue
        stderr_lines.append(line)
    if stderr_lines:
        print("\n".join(stderr_lines), file=sys.stderr)

    if proc.returncode != 0:
        raise RuntimeError(f"command failed ({proc.returncode}): {' '.join(args)}")


def print_kv(label: str, value: Any):
    print(label, value)
