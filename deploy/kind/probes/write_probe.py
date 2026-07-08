# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import io
import os
import re
import struct
import sys
import time
from dataclasses import dataclass

from probe_common import pg_connect, quote_ident, require, require_equal


@dataclass
class WriteMetrics:
    rows: int = 0
    retries: int = 0
    conflicts: int = 0


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def cfg() -> dict[str, int | str]:
    return {
        "workers": int_env("WRITE_WORKERS", 4),
        "independent_rows": int_env("WRITE_INDEPENDENT_ROWS", 25),
        "shared_batches": int_env("WRITE_SHARED_BATCHES", 4),
        "shared_batch_rows": int_env("WRITE_SHARED_BATCH_ROWS", 10),
        "max_retries": int_env("WRITE_MAX_RETRIES", 8),
        "shared_table": os.environ.get("WRITE_SHARED_TABLE", "write_shared"),
        "independent_prefix": os.environ.get(
            "WRITE_INDEPENDENT_PREFIX", "write_independent"
        ),
    }


def worker_index() -> int:
    raw = os.environ.get("JOB_COMPLETION_INDEX") or os.environ.get("WORKER_INDEX")
    require(raw is not None, "missing JOB_COMPLETION_INDEX/WORKER_INDEX")
    return int(raw)


def table_ref(table: str) -> str:
    return f"public.{quote_ident(table)}"


def point_wkb_hex(x: float, y: float) -> str:
    return struct.pack("<BIdd", 1, 1, x, y).hex()


def copy_bytea_hex(hex_wkb: str) -> str:
    return "\\\\x" + hex_wkb.lower()


def create_table_sql(table: str) -> str:
    return (
        f"CREATE TABLE {table_ref(table)} ("
        "id INT, worker INT, batch INT, name TEXT, geom BINARY)"
    )


def copy_rows(table: str, rows: list[tuple[int, int, int, str, str]]) -> None:
    data = "".join(
        "\t".join([str(row_id), str(worker), str(batch), name, copy_bytea_hex(geom_hex)])
        + "\n"
        for row_id, worker, batch, name, geom_hex in rows
    )
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.copy_expert(
                f"COPY {table_ref(table)} (id, worker, batch, name, geom) FROM STDIN",
                io.StringIO(data),
            )
    finally:
        conn.close()


def setup() -> int:
    settings = cfg()
    workers = int(settings["workers"])
    shared_table = str(settings["shared_table"])
    independent_prefix = str(settings["independent_prefix"])

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(shared_table)}")
            for index in range(workers):
                cur.execute(f"DROP TABLE IF EXISTS {table_ref(f'{independent_prefix}_{index}')}")
            cur.execute(create_table_sql(shared_table))
    finally:
        conn.close()

    print(
        "write_setup",
        f"shared_table=public.{shared_table}",
        f"workers={workers}",
    )
    print("write_setup_ok", True)
    return 0


def independent_rows(index: int, count: int) -> list[tuple[int, int, int, str, str]]:
    return [
        (
            index * 1_000_000 + row,
            index,
            0,
            f"independent_{index}_{row}",
            point_wkb_hex(index * 100.0 + row, float(index)),
        )
        for row in range(count)
    ]


def shared_rows(index: int, batch: int, count: int) -> list[tuple[int, int, int, str, str]]:
    return [
        (
            index * 1_000_000 + batch * 10_000 + row,
            index,
            batch,
            f"shared_{index}_{batch}_{row}",
            point_wkb_hex(index * 100.0 + row, float(batch)),
        )
        for row in range(count)
    ]


def retryable_copy(table: str, rows: list[tuple[int, int, int, str, str]], max_retries: int) -> WriteMetrics:
    metrics = WriteMetrics()
    delay = 0.1
    for attempt in range(max_retries + 1):
        try:
            copy_rows(table, rows)
            metrics.rows = len(rows)
            return metrics
        except Exception as err:  # noqa: BLE001 - probe reports any write failure evidence.
            message = str(err)
            if re.search(r"conflict|concurrent|snapshot|stale|version|transaction", message, re.I):
                metrics.conflicts += 1
            if attempt >= max_retries:
                raise
            metrics.retries += 1
            time.sleep(delay)
            delay = min(delay * 2.0, 2.0)
    raise RuntimeError("unreachable retry loop exit")


def worker() -> int:
    settings = cfg()
    index = worker_index()
    require(index < int(settings["workers"]), "worker index exceeds configured workers")
    shared_table = str(settings["shared_table"])
    independent_table = f"{settings['independent_prefix']}_{index}"
    started = time.perf_counter()

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(independent_table)}")
            cur.execute(create_table_sql(independent_table))
    finally:
        conn.close()

    independent = retryable_copy(
        independent_table,
        independent_rows(index, int(settings["independent_rows"])),
        int(settings["max_retries"]),
    )
    shared = WriteMetrics()
    for batch in range(int(settings["shared_batches"])):
        batch_metrics = retryable_copy(
            shared_table,
            shared_rows(index, batch, int(settings["shared_batch_rows"])),
            int(settings["max_retries"]),
        )
        shared.rows += batch_metrics.rows
        shared.retries += batch_metrics.retries
        shared.conflicts += batch_metrics.conflicts

    elapsed_ms = (time.perf_counter() - started) * 1000.0
    print(
        "write_worker",
        f"worker={index}",
        f"independent_table=public.{independent_table}",
        f"independent_rows={independent.rows}",
        f"shared_rows={shared.rows}",
        f"retries={independent.retries + shared.retries}",
        f"conflicts={independent.conflicts + shared.conflicts}",
        f"elapsed_ms={elapsed_ms:.2f}",
    )
    print("write_worker_ok", True)
    return 0


def query_one(sql: str):
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            cur.execute(sql)
            return cur.fetchone()[0]
    finally:
        conn.close()


def compact(table: str) -> None:
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"CALL quackgis_compact_table('public.{table}')")
    finally:
        conn.close()


def verify_spatial_count(table: str, expected: int) -> None:
    count = query_one(
        f"SELECT COUNT(*) FROM {table_ref(table)} "
        "WHERE ST_Intersects("
        "ST_GeomFromWKB(geom), "
        "ST_GeomFromWKB(ST_MakeEnvelope(-1.0, -1.0, 1000000.0, 1000000.0, 3857)))"
    )
    require_equal(count, expected, f"spatial count for public.{table}")


def verify() -> int:
    settings = cfg()
    workers = int(settings["workers"])
    independent_rows_per_worker = int(settings["independent_rows"])
    shared_rows_per_worker = int(settings["shared_batches"]) * int(settings["shared_batch_rows"])
    expected_shared = workers * shared_rows_per_worker
    shared_table = str(settings["shared_table"])
    independent_prefix = str(settings["independent_prefix"])

    for index in range(workers):
        table = f"{independent_prefix}_{index}"
        count = query_one(f"SELECT COUNT(*) FROM {table_ref(table)}")
        require_equal(count, independent_rows_per_worker, f"independent count for {table}")
        verify_spatial_count(table, independent_rows_per_worker)

    shared_count = query_one(f"SELECT COUNT(*) FROM {table_ref(shared_table)}")
    distinct_ids = query_one(f"SELECT COUNT(DISTINCT id) FROM {table_ref(shared_table)}")
    require_equal(shared_count, expected_shared, "shared row count")
    require_equal(distinct_ids, expected_shared, "shared distinct id count")
    verify_spatial_count(shared_table, expected_shared)

    compact(shared_table)
    compacted_count = query_one(f"SELECT COUNT(*) FROM {table_ref(shared_table)}")
    require_equal(compacted_count, expected_shared, "shared row count after compact")

    print(
        "write_verify",
        f"workers={workers}",
        f"independent_tables={workers}",
        f"independent_rows={workers * independent_rows_per_worker}",
        f"shared_rows={shared_count}",
        f"shared_distinct_ids={distinct_ids}",
        f"shared_rows_after_compact={compacted_count}",
    )
    print("write_ok", True)
    return 0


def main() -> int:
    mode = os.environ.get("WRITE_MODE", "verify")
    if mode == "setup":
        return setup()
    if mode == "worker":
        return worker()
    if mode == "verify":
        return verify()
    raise RuntimeError(f"unsupported WRITE_MODE {mode!r}")


if __name__ == "__main__":
    sys.exit(main())
