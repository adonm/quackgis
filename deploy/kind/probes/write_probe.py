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


CONFLICT_RE = re.compile(r"conflict|concurrent|snapshot|stale|version|transaction", re.I)


@dataclass
class WriteMetrics:
    rows: int = 0
    retries: int = 0
    conflicts: int = 0


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def float_env(name: str, default: float) -> float:
    value = float(os.environ.get(name, str(default)))
    require(value > 0.0, f"{name} must be positive")
    return value


def cfg() -> dict[str, int | float | str]:
    return {
        "workers": int_env("WRITE_WORKERS", 4),
        "independent_rows": int_env("WRITE_INDEPENDENT_ROWS", 25),
        "shared_batches": int_env("WRITE_SHARED_BATCHES", 4),
        "shared_batch_rows": int_env("WRITE_SHARED_BATCH_ROWS", 10),
        "max_retries": int_env("WRITE_MAX_RETRIES", 8),
        "visibility_timeout_secs": float_env("WRITE_VISIBILITY_TIMEOUT_SECS", 30.0),
        "shared_table": os.environ.get("WRITE_SHARED_TABLE", "write_shared"),
        "conflict_table": os.environ.get("WRITE_CONFLICT_TABLE", "write_conflict"),
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
    conflict_table = str(settings["conflict_table"])
    independent_prefix = str(settings["independent_prefix"])

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(shared_table)}")
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(conflict_table)}")
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
            if CONFLICT_RE.search(message):
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


def compact(table: str) -> None:
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"CALL quackgis_compact_table('public.{table}')")
    finally:
        conn.close()


def verify_spatial_count(table: str, expected: int, timeout_secs: float) -> None:
    wait_for_query_rows(
        f"SELECT COUNT(*) FROM {table_ref(table)} "
        "WHERE ST_Intersects("
        "ST_GeomFromWKB(geom), "
        "ST_GeomFromWKB(ST_MakeEnvelope(-1.0, -1.0, 1000000.0, 1000000.0, 3857)))",
        [(expected,)],
        f"spatial count for public.{table}",
        timeout_secs,
    )


def query_rows(sql: str):
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            cur.execute(sql)
            return cur.fetchall()
    finally:
        conn.close()


def wait_for_query_rows(
    sql: str, expected: list[tuple], label: str, timeout_secs: float
) -> list[tuple]:
    """Poll service reads until the shared snapshot is visible."""

    deadline = time.monotonic() + timeout_secs
    last_result = None
    while True:
        try:
            rows = query_rows(sql)
        except Exception as err:  # noqa: BLE001 - report transient read/catalog failures.
            last_result = repr(err)
        else:
            if rows == expected:
                return rows
            last_result = rows

        if time.monotonic() >= deadline:
            raise RuntimeError(
                f"timed out waiting for {label}: last={last_result!r}, expected={expected!r}"
            )
        time.sleep(0.25)


def commit_conflict_message(table: str) -> str:
    """Force one stale transactional update over a concurrent autocommit write."""

    conn1 = pg_connect()
    conn2 = pg_connect()
    conn1.autocommit = True
    conn2.autocommit = True
    try:
        with conn1.cursor() as cur1, conn2.cursor() as cur2:
            cur1.execute("BEGIN")
            cur1.execute(f"UPDATE {table_ref(table)} SET label = 'staged' WHERE id = 1")
            cur2.execute(f"INSERT INTO {table_ref(table)} VALUES (2, 'concurrent')")
            try:
                cur1.execute("COMMIT")
            except Exception as err:  # noqa: BLE001 - probe reports boundary error text.
                message = str(err)
                require(
                    CONFLICT_RE.search(message) is not None,
                    f"commit failed without snapshot-conflict evidence: {message}",
                )
                return message
    finally:
        conn1.close()
        conn2.close()

    raise RuntimeError("expected stale transaction COMMIT to fail")


def retry_transactional_update(table: str, max_retries: int) -> int:
    delay = 0.1
    for attempt in range(1, max_retries + 2):
        conn = pg_connect()
        conn.autocommit = True
        try:
            with conn.cursor() as cur:
                cur.execute("BEGIN")
                cur.execute(f"UPDATE {table_ref(table)} SET label = 'retried' WHERE id = 1")
                cur.execute("COMMIT")
                return attempt
        except Exception as err:  # noqa: BLE001 - retry policy is intentionally broad in probe code.
            message = str(err)
            if attempt > max_retries or not CONFLICT_RE.search(message):
                raise
            time.sleep(delay)
            delay = min(delay * 2.0, 2.0)
        finally:
            conn.close()
    raise RuntimeError("unreachable retry loop exit")


def verify_snapshot_conflict_retry(table: str, max_retries: int, timeout_secs: float) -> None:
    """Document the Alpha write contract with executable shared-catalog evidence.

    The first connection stages a transactional update, the second connection
    publishes a newer DuckLake snapshot, then the first COMMIT must fail closed.
    A fresh transaction retries against the newer snapshot and must preserve the
    concurrent row.
    """

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(table)}")
            cur.execute(f"CREATE TABLE {table_ref(table)} (id INT, label TEXT)")
            cur.execute(f"INSERT INTO {table_ref(table)} VALUES (1, 'base')")
    finally:
        conn.close()

    wait_for_query_rows(
        f"SELECT id, label FROM {table_ref(table)} ORDER BY id",
        [(1, "base")],
        "base conflict row visibility",
        timeout_secs,
    )

    conflict_message = commit_conflict_message(table)
    rows_after_conflict = wait_for_query_rows(
        f"SELECT id, label FROM {table_ref(table)} ORDER BY id",
        [(1, "base"), (2, "concurrent")],
        "rows after stale transaction conflict",
        timeout_secs,
    )
    require_equal(
        rows_after_conflict,
        [(1, "base"), (2, "concurrent")],
        "rows after stale transaction conflict",
    )

    retry_attempts = retry_transactional_update(table, max_retries)
    rows_after_retry = wait_for_query_rows(
        f"SELECT id, label FROM {table_ref(table)} ORDER BY id",
        [(1, "retried"), (2, "concurrent")],
        "rows after retrying conflicted transaction",
        timeout_secs,
    )
    require_equal(
        rows_after_retry,
        [(1, "retried"), (2, "concurrent")],
        "rows after retrying conflicted transaction",
    )

    print(
        "write_conflict",
        f"table=public.{table}",
        "conflict_observed=True",
        "failed_commits=1",
        f"retry_attempts={retry_attempts}",
        f"conflict_message={conflict_message.strip().splitlines()[0]!r}",
    )


def verify() -> int:
    settings = cfg()
    workers = int(settings["workers"])
    independent_rows_per_worker = int(settings["independent_rows"])
    shared_rows_per_worker = int(settings["shared_batches"]) * int(settings["shared_batch_rows"])
    expected_shared = workers * shared_rows_per_worker
    shared_table = str(settings["shared_table"])
    conflict_table = str(settings["conflict_table"])
    independent_prefix = str(settings["independent_prefix"])
    visibility_timeout_secs = float(settings["visibility_timeout_secs"])

    for index in range(workers):
        table = f"{independent_prefix}_{index}"
        count = wait_for_query_rows(
            f"SELECT COUNT(*) FROM {table_ref(table)}",
            [(independent_rows_per_worker,)],
            f"independent count for {table}",
            visibility_timeout_secs,
        )[0][0]
        require_equal(count, independent_rows_per_worker, f"independent count for {table}")
        verify_spatial_count(table, independent_rows_per_worker, visibility_timeout_secs)

    shared_count, distinct_ids = wait_for_query_rows(
        f"SELECT COUNT(*), COUNT(DISTINCT id) FROM {table_ref(shared_table)}",
        [(expected_shared, expected_shared)],
        "shared row/distinct id count",
        visibility_timeout_secs,
    )[0]
    require_equal(shared_count, expected_shared, "shared row count")
    require_equal(distinct_ids, expected_shared, "shared distinct id count")
    verify_spatial_count(shared_table, expected_shared, visibility_timeout_secs)

    compact(shared_table)
    compacted_count = wait_for_query_rows(
        f"SELECT COUNT(*) FROM {table_ref(shared_table)}",
        [(expected_shared,)],
        "shared row count after compact",
        visibility_timeout_secs,
    )[0][0]
    require_equal(compacted_count, expected_shared, "shared row count after compact")
    verify_snapshot_conflict_retry(
        conflict_table, int(settings["max_retries"]), visibility_timeout_secs
    )

    print(
        "write_verify",
        f"workers={workers}",
        f"independent_tables={workers}",
        f"independent_rows={workers * independent_rows_per_worker}",
        f"shared_rows={shared_count}",
        f"shared_distinct_ids={distinct_ids}",
        f"shared_rows_after_compact={compacted_count}",
        f"conflict_table=public.{conflict_table}",
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
