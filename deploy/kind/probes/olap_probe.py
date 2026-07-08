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
from decimal import Decimal

from probe_common import pg_connect, quackgis_host, quackgis_port, quote_ident, require, require_equal


@dataclass(frozen=True)
class Rect:
    minx: float
    miny: float
    maxx: float
    maxy: float


@dataclass(frozen=True)
class AssetRow:
    row_id: int
    mission: str
    asset_class: str
    zone: int
    capture_day: int
    quality: float
    area_m2: float
    risk_score: float
    risk_flag: int
    footprint: Rect


@dataclass
class GroupStats:
    count: int = 0
    area_total: float = 0.0
    quality_total: float = 0.0
    high_risk: int = 0

    @property
    def quality_avg(self) -> float:
        require(self.count > 0, "cannot average empty group")
        return self.quality_total / self.count


@dataclass(frozen=True)
class Sample:
    elapsed_ms: float


OLAP_WINDOW = Rect(560.0, 72.0, 1_760.0, 302.0)
CAPTURE_DAY_MIN = 18
CAPTURE_DAY_MAX = 155
QUALITY_MIN = 0.70
RISK_FLAG_THRESHOLD = 58.0


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def table_ref(table: str) -> str:
    return f"public.{quote_ident(table)}"


def internal_table_ref(table: str) -> str:
    return f"quackgis.main.{quote_ident(table)}"


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def rect_intersects(left: Rect, right: Rect) -> bool:
    return (
        left.minx <= right.maxx
        and left.maxx >= right.minx
        and left.miny <= right.maxy
        and left.maxy >= right.miny
    )


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


def asset_rows(factor: int) -> list[AssetRow]:
    rows: list[AssetRow] = []
    row_id = 1
    classes = ["roof", "road", "utility"]
    for block in range(factor):
        for mission_idx in range(6):
            for zone in range(8):
                for class_idx, asset_class in enumerate(classes):
                    width = 14.0 + (mission_idx % 3) * 3.0 + class_idx
                    height = 10.0 + (zone % 4) * 2.5 + class_idx * 0.5
                    minx = block * 185.0 + mission_idx * 24.0 + class_idx * 5.0
                    miny = zone * 38.0 + class_idx * 7.0 + (block % 4) * 1.5
                    footprint = Rect(minx, miny, minx + width, miny + height)
                    quality = 0.63 + (mission_idx % 4) * 0.045 + class_idx * 0.025 - (zone % 3) * 0.015
                    risk_score = 35.0 + zone * 5.5 + class_idx * 4.0 + (block % 6) * 2.0
                    rows.append(
                        AssetRow(
                            row_id=row_id,
                            mission=f"mission_{mission_idx:02}",
                            asset_class=asset_class,
                            zone=zone,
                            capture_day=block * 4 + mission_idx * 3 + zone,
                            quality=quality,
                            area_m2=width * height,
                            risk_score=risk_score,
                            risk_flag=1 if risk_score >= RISK_FLAG_THRESHOLD else 0,
                            footprint=footprint,
                        )
                    )
                    row_id += 1
    return rows


def create_and_seed(table: str, factor: int) -> int:
    rows = asset_rows(factor)
    copy_data = "".join(
        "\t".join(
            [
                str(row.row_id),
                row.mission,
                row.asset_class,
                str(row.zone),
                str(row.capture_day),
                f"{row.quality:.6f}",
                f"{row.area_m2:.6f}",
                f"{row.risk_score:.6f}",
                str(row.risk_flag),
                copy_bytea_hex(rect_wkb_hex(row.footprint)),
            ]
        )
        + "\n"
        for row in rows
    )

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref(table)}")
            cur.execute(
                f"CREATE TABLE {table_ref(table)} ("
                "id INT, mission TEXT, asset_class TEXT, zone INT, capture_day INT, "
                "quality DOUBLE, area_m2 DOUBLE, risk_score DOUBLE, risk_flag INT, geom BINARY)"
            )
            cur.copy_expert(
                f"COPY {table_ref(table)} "
                "(id, mission, asset_class, zone, capture_day, quality, area_m2, risk_score, risk_flag, geom) "
                "FROM STDIN",
                io.StringIO(copy_data),
            )
            cur.execute(f"CALL quackgis_compact_table('public.{table}')")
    finally:
        conn.close()
    return len(rows)


def spatial_predicate(geom_expr: str) -> str:
    return (
        f"ST_Intersects(ST_GeomFromWKB({geom_expr}), "
        f"ST_GeomFromWKB(ST_MakeEnvelope({OLAP_WINDOW.minx}, {OLAP_WINDOW.miny}, "
        f"{OLAP_WINDOW.maxx}, {OLAP_WINDOW.maxy}, 3857)))"
    )


def attribute_predicate() -> str:
    return (
        f"capture_day BETWEEN {CAPTURE_DAY_MIN} AND {CAPTURE_DAY_MAX} "
        f"AND quality >= {QUALITY_MIN}"
    )


def hidden_bbox_predicate() -> str:
    return (
        f"_qg_minx <= {OLAP_WINDOW.maxx} AND _qg_maxx >= {OLAP_WINDOW.minx} "
        f"AND _qg_miny <= {OLAP_WINDOW.maxy} AND _qg_maxy >= {OLAP_WINDOW.miny}"
    )


def aggregate_sql(table: str, *, internal: bool) -> str:
    source = internal_table_ref(table) if internal else table_ref(table)
    predicates = [attribute_predicate()]
    if internal:
        predicates.append(hidden_bbox_predicate())
    predicates.append(spatial_predicate(quote_ident("geom")))
    where_clause = " AND ".join(predicates)
    return (
        "SELECT mission, asset_class, COUNT(*) AS features, "
        "SUM(area_m2) AS area_total, AVG(quality) AS quality_avg, SUM(risk_flag) AS high_risk "
        f"FROM {source} "
        f"WHERE {where_clause} "
        "GROUP BY mission, asset_class "
        "ORDER BY mission, asset_class"
    )


def expected_groups(rows: list[AssetRow]) -> dict[tuple[str, str], GroupStats]:
    groups: dict[tuple[str, str], GroupStats] = {}
    for row in rows:
        if not (CAPTURE_DAY_MIN <= row.capture_day <= CAPTURE_DAY_MAX):
            continue
        if row.quality < QUALITY_MIN:
            continue
        if not rect_intersects(row.footprint, OLAP_WINDOW):
            continue
        key = (row.mission, row.asset_class)
        group = groups.setdefault(key, GroupStats())
        group.count += 1
        group.area_total += row.area_m2
        group.quality_total += row.quality
        group.high_risk += row.risk_flag
    return groups


def fetch_rows(sql: str):
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            cur.execute(sql)
            return cur.fetchall()
    finally:
        conn.close()


def to_float(value) -> float:
    if isinstance(value, Decimal):
        return float(value)
    return float(value)


def require_close(actual, expected: float, label: str) -> None:
    actual_float = to_float(actual)
    tolerance = max(1.0, abs(expected)) * 1.0e-8
    if abs(actual_float - expected) > tolerance:
        raise RuntimeError(f"unexpected {label}: {actual_float!r} != {expected!r}")


def assert_group_rows(rows, expected: dict[tuple[str, str], GroupStats], label: str) -> None:
    actual_keys = {(mission, asset_class) for mission, asset_class, *_ in rows}
    require_equal(actual_keys, set(expected), f"{label} group keys")
    for mission, asset_class, count, area_total, quality_avg, high_risk in rows:
        group = expected[(mission, asset_class)]
        key_label = f"{label} {mission}/{asset_class}"
        require_equal(int(count), group.count, f"{key_label} count")
        require_close(area_total, group.area_total, f"{key_label} area_total")
        require_close(quality_avg, group.quality_avg, f"{key_label} quality_avg")
        require_equal(int(high_risk), group.high_risk, f"{key_label} high_risk")


def candidate_groups(groups: dict[tuple[str, str], GroupStats]) -> list[tuple[str, str]]:
    candidates = [
        (key, group)
        for key, group in groups.items()
        if group.count >= 2 and group.high_risk >= 1 and group.area_total >= 350.0
    ]
    candidates.sort(key=lambda item: (-item[1].area_total, item[0][0], item[0][1]))
    return [key for key, _group in candidates[:6]]


def candidate_recheck_sql(table: str, candidates: list[tuple[str, str]]) -> str:
    candidate_predicate = " OR ".join(
        f"(mission = {sql_literal(mission)} AND asset_class = {sql_literal(asset_class)})"
        for mission, asset_class in candidates
    )
    require(candidate_predicate, "candidate predicate must not be empty")
    return (
        "SELECT COUNT(*) AS features, SUM(area_m2) AS area_total "
        f"FROM {table_ref(table)} "
        f"WHERE {attribute_predicate()} "
        f"AND {spatial_predicate(quote_ident('geom'))} "
        f"AND ({candidate_predicate})"
    )


def explain_analyze(sql: str) -> str:
    rows = simple_query_rows(f"EXPLAIN ANALYZE {sql}")
    return "\n".join("\n".join(value for value in row if value) for row in rows if row)


def simple_query_rows(sql: str) -> list[list[str | None]]:
    """Run one PostgreSQL simple-query message and return text DataRow values."""

    with socket.create_connection((quackgis_host(), quackgis_port()), timeout=30) as sock:
        sock.settimeout(180)
        startup_params = (
            b"user\0postgres\0database\0quackgis\0client_encoding\0UTF8\0"
            b"application_name\0olap_probe\0\0"
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


def scan_summary(plan: str) -> dict[str, object]:
    return {
        "output_rows": metric_value(plan, "output_rows"),
        "bytes_scanned": metric_value(plan, "bytes_scanned"),
        "hidden_bbox": all(
            token in plan for token in ("_qg_minx", "_qg_maxx", "_qg_miny", "_qg_maxy")
        ),
        "parquet_predicate": "DataSourceExec" in plan and "predicate=" in plan,
        "aggregate_exec": "AggregateExec" in plan or "Aggregate" in plan,
        "projection_evidence": "projection=" in plan or "required_columns" in plan,
        "row_groups_pruned_statistics": metric_value(plan, "row_groups_pruned_statistics"),
        "files_ranges_pruned_statistics": metric_value(plan, "files_ranges_pruned_statistics"),
    }


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


def run_worker(worker_id: int, assigned_queries: int, sql: str, expected: dict[tuple[str, str], GroupStats]):
    samples: list[Sample] = []
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            for _ in range(assigned_queries):
                started = time.perf_counter()
                cur.execute(sql)
                rows = cur.fetchall()
                elapsed_ms = (time.perf_counter() - started) * 1000.0
                assert_group_rows(rows, expected, f"worker {worker_id} aggregate")
                samples.append(Sample(elapsed_ms))
    finally:
        conn.close()
    return samples


def run_probe(table: str, factor: int, workers: int, total_queries: int, seeded_rows: int | str) -> int:
    expected = expected_groups(asset_rows(factor))
    require(expected, "OLAP fanout expected group set must not be empty")

    public_sql = aggregate_sql(table, internal=False)
    internal_sql = aggregate_sql(table, internal=True)
    public_rows = fetch_rows(public_sql)
    internal_rows = fetch_rows(internal_sql)
    assert_group_rows(public_rows, expected, "public exact aggregate")
    assert_group_rows(internal_rows, expected, "internal pruned aggregate")

    plan = explain_analyze(internal_sql)
    scan = scan_summary(plan)
    require(scan["hidden_bbox"], "EXPLAIN plan did not include hidden bbox predicate")
    require(scan["aggregate_exec"], "EXPLAIN plan did not include aggregate execution")

    candidates = candidate_groups(expected)
    require(candidates, "OLAP fanout candidate groups must not be empty")
    candidate_expected_count = sum(expected[key].count for key in candidates)
    candidate_expected_area = sum(expected[key].area_total for key in candidates)
    candidate_count, candidate_area = fetch_rows(candidate_recheck_sql(table, candidates))[0]
    require_equal(int(candidate_count), candidate_expected_count, "candidate exact recheck count")
    require_close(candidate_area, candidate_expected_area, "candidate exact recheck area")

    print(
        "olap_config",
        f"table=public.{table}",
        f"seeded_rows={seeded_rows}",
        f"factor={factor}",
        f"workers={workers}",
        f"queries={total_queries}",
        f"window={OLAP_WINDOW.minx},{OLAP_WINDOW.miny},{OLAP_WINDOW.maxx},{OLAP_WINDOW.maxy}",
    )
    print(
        "olap_scan",
        f"groups={len(expected)}",
        f"output_rows={scan['output_rows']}",
        f"bytes_scanned={scan['bytes_scanned']}",
        f"row_groups_pruned_statistics={scan['row_groups_pruned_statistics']}",
        f"files_ranges_pruned_statistics={scan['files_ranges_pruned_statistics']}",
        f"hidden_bbox={scan['hidden_bbox']}",
        f"parquet_predicate={scan['parquet_predicate']}",
        f"aggregate_exec={scan['aggregate_exec']}",
        f"projection_evidence={scan['projection_evidence']}",
    )
    print(
        "olap_recheck",
        f"candidate_groups={len(candidates)}",
        f"candidate_rows={candidate_count}",
        f"candidate_area={to_float(candidate_area):.2f}",
    )

    started = time.perf_counter()
    assignments = [total_queries // workers] * workers
    for idx in range(total_queries % workers):
        assignments[idx] += 1
    samples: list[Sample] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [
            executor.submit(run_worker, worker_id, assigned, public_sql, expected)
            for worker_id, assigned in enumerate(assignments)
            if assigned > 0
        ]
        for future in concurrent.futures.as_completed(futures):
            samples.extend(future.result())
    elapsed_s = time.perf_counter() - started
    require_equal(len(samples), total_queries, "completed OLAP query count")
    summary = latency_summary(samples)
    qps = total_queries / elapsed_s if elapsed_s else 0.0
    print(
        "olap_result",
        f"queries={total_queries}",
        f"workers={workers}",
        f"elapsed_ms={elapsed_s * 1000.0:.2f}",
        f"qps={qps:.2f}",
        f"avg_ms={summary['avg_ms']:.2f}",
        f"p50_ms={summary['p50_ms']:.2f}",
        f"p95_ms={summary['p95_ms']:.2f}",
        f"p99_ms={summary['p99_ms']:.2f}",
    )
    print("olap_ok", True)
    return 0


def main() -> int:
    mode = os.environ.get("OLAP_MODE", "seed-probe")
    factor = int_env("OLAP_FACTOR", 30)
    workers = int_env("OLAP_WORKERS", 4)
    total_queries = int_env("OLAP_QUERIES", 24)
    table = os.environ.get("OLAP_TABLE", "olap_assets")

    rows: int | str = "preseeded"
    if mode in ("seed", "seed-probe"):
        rows = create_and_seed(table, factor)
        print("olap_seed", f"table=public.{table}", f"rows={rows}", f"factor={factor}")
        if mode == "seed":
            print("olap_seed_ok", True)
            return 0

    require(mode in ("probe", "seed-probe"), f"unsupported OLAP_MODE {mode!r}")
    return run_probe(table, factor, workers, total_queries, rows)


if __name__ == "__main__":
    sys.exit(main())
