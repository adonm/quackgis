# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import concurrent.futures
import io
import os
import re
import socket
import statistics
import struct
import sys
import time
from dataclasses import dataclass

from probe_common import (
    pg_connect,
    quackgis_host,
    quackgis_port,
    quote_ident,
    require,
    require_equal,
    table_name,
)


@dataclass(frozen=True)
class Rect:
    minx: float
    miny: float
    maxx: float
    maxy: float


@dataclass(frozen=True)
class QueryCase:
    label: str
    geom_column: str
    envelope: Rect
    extra_predicate: str


@dataclass(frozen=True)
class Sample:
    label: str
    elapsed_ms: float


AERIAL_CASE = QueryCase(
    label="aerial",
    geom_column="geom",
    envelope=Rect(95.0, 95.0, 290.0, 185.0),
    extra_predicate="captured_minute BETWEEN 40 AND 170",
)


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def rect_wkb_hex(rect: Rect) -> str:
    points = [
        (rect.minx, rect.miny),
        (rect.maxx, rect.miny),
        (rect.maxx, rect.maxy),
        (rect.minx, rect.maxy),
        (rect.minx, rect.miny),
    ]
    out = bytearray()
    out.extend(struct.pack("<BII", 1, 3, 1))  # little-endian Polygon, one ring
    out.extend(struct.pack("<I", len(points)))
    for x, y in points:
        out.extend(struct.pack("<dd", x, y))
    return out.hex()


def copy_bytea_hex(hex_wkb: str) -> str:
    return "\\\\x" + hex_wkb.lower()


def aerial_rows(factor: int):
    row_id = 1
    for block in range(factor):
        for strip in range(6):
            for frame in range(18):
                minx = block * 420.0 + strip * 42.0 + frame * 4.0
                miny = strip * 36.0 + (frame % 4) * 8.0
                footprint = Rect(
                    minx=minx,
                    miny=miny,
                    maxx=minx + 28.0,
                    maxy=miny + 22.0,
                )
                yield (
                    row_id,
                    f"mission_{block:03}_{strip:02}",
                    strip,
                    block * 240 + strip * 20 + frame,
                    3.5 + (strip % 3) * 0.5,
                    800.0 + strip * 25.0,
                    footprint,
                )
                row_id += 1


def create_and_seed(table: str, factor: int) -> int:
    table_ref = f"public.{quote_ident(table)}"
    rows = list(aerial_rows(factor))
    copy_data = "".join(
        "\t".join(
            [
                str(row_id),
                mission,
                str(strip),
                str(captured_minute),
                str(gsd_cm),
                str(altitude_m),
                copy_bytea_hex(rect_wkb_hex(footprint)),
            ]
        )
        + "\n"
        for row_id, mission, strip, captured_minute, gsd_cm, altitude_m, footprint in rows
    )

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref}")
            cur.execute(
                f"CREATE TABLE {table_ref} ("
                "id INT, mission TEXT, strip INT, captured_minute INT, "
                "gsd_cm DOUBLE, altitude_m DOUBLE, geom BINARY)"
            )
            cur.copy_expert(
                f"COPY {table_ref} "
                "(id, mission, strip, captured_minute, gsd_cm, altitude_m, geom) "
                "FROM STDIN",
                io.StringIO(copy_data),
            )
            cur.execute(f"CALL quackgis_compact_table('public.{table}')")
    finally:
        conn.close()
    return len(rows)


def exact_sql(table: str, case: QueryCase) -> str:
    return (
        f"SELECT COUNT(*) AS n FROM public.{quote_ident(table)} "
        f"WHERE {case.extra_predicate} "
        f"AND ST_Intersects(ST_GeomFromWKB({quote_ident(case.geom_column)}), "
        f"ST_GeomFromWKB(ST_MakeEnvelope({case.envelope.minx}, {case.envelope.miny}, "
        f"{case.envelope.maxx}, {case.envelope.maxy}, 3857)))"
    )


def scan_evidence_sql(table: str, case: QueryCase) -> str:
    return (
        f"SELECT COUNT(*) AS n FROM quackgis.main.{quote_ident(table)} "
        f"WHERE {case.extra_predicate} "
        f"AND _qg_minx <= {case.envelope.maxx} AND _qg_maxx >= {case.envelope.minx} "
        f"AND _qg_miny <= {case.envelope.maxy} AND _qg_maxy >= {case.envelope.miny} "
        f"AND ST_Intersects(ST_GeomFromWKB({quote_ident(case.geom_column)}), "
        f"ST_GeomFromWKB(ST_MakeEnvelope({case.envelope.minx}, {case.envelope.miny}, "
        f"{case.envelope.maxx}, {case.envelope.maxy}, 3857)))"
    )


def query_count(sql: str) -> int:
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            cur.execute(sql)
            return cur.fetchone()[0]
    finally:
        conn.close()


def explain_analyze(sql: str) -> str:
    rows = simple_query_rows(f"EXPLAIN ANALYZE {sql}")
    return "\n".join("\n".join(value for value in row if value) for row in rows if row)


def simple_query_rows(sql: str) -> list[list[str | None]]:
    """Run one PostgreSQL simple-query message and return text DataRow values.

    psycopg2 uses extended protocol for `execute()`, and this stack currently
    returns only a terse `Plan with Metrics` row for extended EXPLAIN. The probe
    needs the full simple-query EXPLAIN text to assert scan/pruning evidence.
    """

    with socket.create_connection((quackgis_host(), quackgis_port()), timeout=30) as sock:
        sock.settimeout(120)
        startup_params = (
            b"user\0postgres\0database\0quackgis\0client_encoding\0UTF8\0"
            b"application_name\0read_probe\0\0"
        )
        sock.sendall(struct.pack("!II", len(startup_params) + 8, 196608) + startup_params)
        read_until_ready(sock)

        payload = sql.encode("utf-8") + b"\0"
        sock.sendall(b"Q" + struct.pack("!I", len(payload) + 4) + payload)
        rows: list[list[str | None]] = []
        while True:
            message_type, message = read_pg_message(sock)
            if message_type == b"D":
                rows.append(parse_data_row(message))
            elif message_type == b"E":
                raise RuntimeError(pg_error_message(message))
            elif message_type == b"Z":
                return rows


def read_until_ready(sock: socket.socket) -> None:
    while True:
        message_type, message = read_pg_message(sock)
        if message_type == b"R":
            auth_code = struct.unpack("!I", message[:4])[0]
            require(auth_code == 0, f"unsupported PostgreSQL auth request {auth_code}")
        elif message_type == b"E":
            raise RuntimeError(pg_error_message(message))
        elif message_type == b"Z":
            return


def read_pg_message(sock: socket.socket) -> tuple[bytes, bytes]:
    message_type = read_exact(sock, 1)
    length = struct.unpack("!I", read_exact(sock, 4))[0]
    require(length >= 4, f"invalid PostgreSQL message length {length}")
    return message_type, read_exact(sock, length - 4)


def read_exact(sock: socket.socket, length: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < length:
        chunk = sock.recv(length - len(chunks))
        if not chunk:
            raise RuntimeError("PostgreSQL connection closed unexpectedly")
        chunks.extend(chunk)
    return bytes(chunks)


def parse_data_row(message: bytes) -> list[str | None]:
    field_count = struct.unpack("!H", message[:2])[0]
    offset = 2
    row: list[str | None] = []
    for _ in range(field_count):
        field_len = struct.unpack("!i", message[offset : offset + 4])[0]
        offset += 4
        if field_len < 0:
            row.append(None)
            continue
        value = message[offset : offset + field_len]
        offset += field_len
        row.append(value.decode("utf-8", errors="replace"))
    return row


def pg_error_message(message: bytes) -> str:
    fields = []
    offset = 0
    while offset < len(message) and message[offset] != 0:
        code = chr(message[offset])
        offset += 1
        end = message.find(b"\0", offset)
        if end < 0:
            break
        value = message[offset:end].decode("utf-8", errors="replace")
        if code in ("S", "C", "M", "D", "H"):
            fields.append(value)
        offset = end + 1
    return ": ".join(fields) or "PostgreSQL error"


def metric_value(plan: str, metric_name: str):
    match = re.search(rf"{re.escape(metric_name)}=\s*([0-9,]+)", plan)
    if not match:
        return "NA"
    return match.group(1).replace(",", "")


def file_group_count(plan: str):
    matches = re.findall(r"file_groups=\{([0-9,]+)\s+groups?", plan)
    if not matches:
        return "NA"
    return str(max(int(value.replace(",", "")) for value in matches))


def scan_summary(plan: str) -> dict[str, object]:
    return {
        "output_rows": metric_value(plan, "output_rows"),
        "bytes_scanned": metric_value(plan, "bytes_scanned"),
        "file_groups": file_group_count(plan),
        "hidden_bbox": all(
            token in plan for token in ("_qg_minx", "_qg_maxx", "_qg_miny", "_qg_maxy")
        ),
        "parquet_predicate": "DataSourceExec" in plan and "predicate=" in plan,
        "row_groups_pruned_statistics": metric_value(plan, "row_groups_pruned_statistics"),
        "files_ranges_pruned_statistics": metric_value(plan, "files_ranges_pruned_statistics"),
    }


def run_worker(worker_id: int, assigned_queries: int, sql: str, expected_count: int):
    samples = []
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            for _ in range(assigned_queries):
                started = time.perf_counter()
                cur.execute(sql)
                count = cur.fetchone()[0]
                elapsed_ms = (time.perf_counter() - started) * 1000.0
                require_equal(count, expected_count, f"worker {worker_id} count")
                samples.append(Sample(AERIAL_CASE.label, elapsed_ms))
    finally:
        conn.close()
    return samples


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    rank = max(1, int(len(values) * fraction + 0.999999))
    return values[min(rank - 1, len(values) - 1)]


def latency_summary(samples: list[Sample]) -> dict[str, float]:
    values = sorted(sample.elapsed_ms for sample in samples)
    return {
        "avg_ms": statistics.fmean(values) if values else 0.0,
        "p50_ms": percentile(values, 0.50),
        "p95_ms": percentile(values, 0.95),
        "p99_ms": percentile(values, 0.99),
    }


def main() -> int:
    mode = os.environ.get("READ_MODE", "seed-read")
    factor = int_env("READ_FACTOR", 25)
    workers = int_env("READ_WORKERS", 8)
    total_queries = int_env("READ_QUERIES", 80)
    table = os.environ.get("READ_TABLE") or table_name("read_aerial")

    rows = 0
    if mode in ("seed", "seed-read"):
        rows = create_and_seed(table, factor)
        print(
            "read_seed",
            f"table=public.{table}",
            f"rows={rows}",
            f"factor={factor}",
        )
        if mode == "seed":
            print("read_seed_ok", True)
            return 0

    require(mode in ("read", "seed-read"), f"unsupported READ_MODE {mode!r}")

    sql = exact_sql(table, AERIAL_CASE)
    expected_count = query_count(sql)
    scan_sql = scan_evidence_sql(table, AERIAL_CASE)
    scan_count = query_count(scan_sql)
    require_equal(scan_count, expected_count, "scan evidence count")
    plan = explain_analyze(sql)
    scan = scan_summary(plan)
    require(expected_count > 0, "read workload expected count must be non-zero")
    require(scan["hidden_bbox"], "EXPLAIN plan did not include injected hidden bbox predicate")

    print(
        "read_config",
        f"table=public.{table}",
        f"seeded_rows={rows if rows else 'preseeded'}",
        f"factor={factor}",
        f"workers={workers}",
        f"queries={total_queries}",
    )
    print(
        "read_scan",
        f"label={AERIAL_CASE.label}",
        f"expected_count={expected_count}",
        f"output_rows={scan['output_rows']}",
        f"bytes_scanned={scan['bytes_scanned']}",
        f"file_groups={scan['file_groups']}",
        f"row_groups_pruned_statistics={scan['row_groups_pruned_statistics']}",
        f"files_ranges_pruned_statistics={scan['files_ranges_pruned_statistics']}",
        f"hidden_bbox={scan['hidden_bbox']}",
        f"parquet_predicate={scan['parquet_predicate']}",
    )

    started = time.perf_counter()
    assignments = [total_queries // workers] * workers
    for idx in range(total_queries % workers):
        assignments[idx] += 1
    all_samples: list[Sample] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [
            executor.submit(run_worker, worker_id, assigned, sql, expected_count)
            for worker_id, assigned in enumerate(assignments)
            if assigned > 0
        ]
        for future in concurrent.futures.as_completed(futures):
            all_samples.extend(future.result())
    elapsed_s = time.perf_counter() - started
    require_equal(len(all_samples), total_queries, "completed query count")
    summary = latency_summary(all_samples)
    qps = total_queries / elapsed_s if elapsed_s else 0.0
    print(
        "read_result",
        f"label={AERIAL_CASE.label}",
        f"queries={total_queries}",
        f"workers={workers}",
        f"elapsed_ms={elapsed_s * 1000.0:.2f}",
        f"qps={qps:.2f}",
        f"avg_ms={summary['avg_ms']:.2f}",
        f"p50_ms={summary['p50_ms']:.2f}",
        f"p95_ms={summary['p95_ms']:.2f}",
        f"p99_ms={summary['p99_ms']:.2f}",
    )
    print("read_ok", True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
