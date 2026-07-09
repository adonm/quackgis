# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import concurrent.futures
import os
import re
import sys
import time
import urllib.request
from collections import Counter
from dataclasses import dataclass

import read_probe
from probe_common import pg_connect, require, require_equal, table_name


@dataclass(frozen=True)
class WorkerResult:
    instance_id: str
    samples: list[read_probe.Sample]


@dataclass(frozen=True)
class QueryPlan:
    case: read_probe.QueryCase
    sql: str
    expected_count: int


@dataclass(frozen=True)
class LinkerdSnapshot:
    open_total: float
    read_bytes: float
    write_bytes: float
    dst_pods: frozenset[str]


ROWS_PER_FACTOR = 6 * 18


class AerialCopyStream:
    """File-like streaming COPY source for large QPS seeds.

    `read_probe.create_and_seed` intentionally keeps the baseline smoke tiny and
    simple. The deep QPS gate can scale several orders of magnitude higher, so it
    must stream rows instead of materializing one giant COPY string in memory.
    """

    def __init__(self, factor: int):
        self._rows = iter(read_probe.aerial_rows(factor))
        self._buffer = ""
        self.count = 0
        self.done = False

    def readable(self) -> bool:
        return True

    def read(self, size: int = -1) -> str:
        if size is None or size < 0:
            chunks = [self._buffer]
            self._buffer = ""
            while not self.done:
                row = self._next_line()
                if row is None:
                    break
                chunks.append(row)
            return "".join(chunks)

        while len(self._buffer) < size and not self.done:
            row = self._next_line()
            if row is None:
                break
            self._buffer += row
        out = self._buffer[:size]
        self._buffer = self._buffer[size:]
        return out

    def _next_line(self) -> str | None:
        try:
            row_id, mission, strip, captured_minute, gsd_cm, altitude_m, footprint = next(self._rows)
        except StopIteration:
            self.done = True
            return None
        self.count += 1
        return (
            "\t".join(
                [
                    str(row_id),
                    mission,
                    str(strip),
                    str(captured_minute),
                    str(gsd_cm),
                    str(altitude_m),
                    read_probe.copy_bytea_hex(read_probe.rect_wkb_hex(footprint)),
                ]
            )
            + "\n"
        )


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def float_env(name: str, default: float) -> float:
    value = float(os.environ.get(name, str(default)))
    require(value >= 0.0, f"{name} must be non-negative")
    return value


def bool_env(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.lower() in ("1", "true", "yes", "on")


def expected_rows(factor: int) -> int:
    return factor * ROWS_PER_FACTOR


def plan_metric_int(scan: dict[str, object], metric: str, label: str) -> int:
    raw = scan[metric]
    require(raw != "NA", f"{label} EXPLAIN plan missed {metric}")
    return int(str(raw))


def block_case(label: str, factor: int, block: int) -> read_probe.QueryCase:
    block = max(0, min(block, factor - 1))
    minx_base = block * 420.0
    minute_base = block * 240
    return read_probe.QueryCase(
        label=label,
        geom_column="geom",
        envelope=read_probe.Rect(minx_base + 95.0, 95.0, minx_base + 290.0, 185.0),
        extra_predicate=f"captured_minute BETWEEN {minute_base + 40} AND {minute_base + 170}",
    )


def qps_cases(factor: int) -> list[read_probe.QueryCase]:
    mid_block = max(0, factor // 2)
    tail_block = max(0, factor - 1)
    corridor_blocks = min(factor, int_env("QPS_CORRIDOR_BLOCKS", 32))
    corridor_max_block = max(0, corridor_blocks - 1)
    return [
        read_probe.AERIAL_CASE,
        read_probe.QueryCase(
            label="origin_swath",
            geom_column="geom",
            envelope=read_probe.Rect(0.0, 0.0, 120.0, 80.0),
            extra_predicate="captured_minute BETWEEN 0 AND 75",
        ),
        block_case("mid_block", factor, mid_block),
        block_case("tail_block", factor, tail_block),
        read_probe.QueryCase(
            label="multi_block_corridor",
            geom_column="geom",
            envelope=read_probe.Rect(0.0, 95.0, corridor_max_block * 420.0 + 290.0, 185.0),
            extra_predicate=f"captured_minute BETWEEN 0 AND {corridor_max_block * 240 + 170}",
        ),
    ]


def create_and_seed(table: str, factor: int, *, compact: bool) -> int:
    table_ref = f"public.{read_probe.quote_ident(table)}"
    stream = AerialCopyStream(factor)
    conn = pg_connect()
    conn.autocommit = True
    started = time.perf_counter()
    try:
        with conn.cursor() as cur:
            cur.execute(f"DROP TABLE IF EXISTS {table_ref}")
            cur.execute(
                f"CREATE TABLE {table_ref} ("
                "id INT, mission TEXT, strip INT, captured_minute INT, "
                "gsd_cm DOUBLE, altitude_m DOUBLE, geom BINARY)"
            )
            load_started = time.perf_counter()
            cur.copy_expert(
                f"COPY {table_ref} "
                "(id, mission, strip, captured_minute, gsd_cm, altitude_m, geom) "
                "FROM STDIN",
                stream,
            )
            print("qps_seed_load", f"rows={stream.count}", f"elapsed_ms={(time.perf_counter() - load_started) * 1000.0:.2f}")
            if compact:
                compact_started = time.perf_counter()
                cur.execute(f"CALL quackgis_compact_table('public.{table}')")
                print("qps_seed_compact", f"elapsed_ms={(time.perf_counter() - compact_started) * 1000.0:.2f}")
    finally:
        conn.close()
    require_equal(stream.count, expected_rows(factor), "streamed seed row count")
    print("qps_seed_elapsed", f"elapsed_ms={(time.perf_counter() - started) * 1000.0:.2f}")
    return stream.count


def query_instance_id(cur) -> str:
    cur.execute("SELECT quackgis_instance_id()")
    value = cur.fetchone()[0]
    require(value, "quackgis_instance_id() returned an empty value")
    return str(value)


def run_worker(worker_id: int, assigned_queries: int, plans: list[QueryPlan]) -> WorkerResult:
    samples: list[read_probe.Sample] = []
    conn = pg_connect()
    try:
        with conn.cursor() as cur:
            instance_id = query_instance_id(cur)
            for query_idx in range(assigned_queries):
                plan = plans[(worker_id + query_idx) % len(plans)]
                started = time.perf_counter()
                cur.execute(plan.sql)
                count = cur.fetchone()[0]
                elapsed_ms = (time.perf_counter() - started) * 1000.0
                require_equal(count, plan.expected_count, f"worker {worker_id} {plan.case.label} count")
                samples.append(read_probe.Sample(plan.case.label, elapsed_ms))
    finally:
        conn.close()
    return WorkerResult(instance_id, samples)


def prepare_query_plans(
    table: str, factor: int, max_file_groups: int, max_bytes_scanned: int
) -> list[QueryPlan]:
    plans: list[QueryPlan] = []
    for case in qps_cases(factor):
        sql = read_probe.exact_sql(table, case)
        expected_count = read_probe.query_count(sql)
        require(expected_count > 0, f"QPS case {case.label} expected count must be non-zero")

        scan_sql = read_probe.scan_evidence_sql(table, case)
        scan_count = read_probe.query_count(scan_sql)
        require_equal(scan_count, expected_count, f"{case.label} scan evidence count")
        plan_text = read_probe.explain_analyze(sql)
        scan = read_probe.scan_summary(plan_text)
        require(scan["hidden_bbox"], f"{case.label} EXPLAIN plan missed hidden bbox predicate")
        file_groups = plan_metric_int(scan, "file_groups", case.label)
        bytes_scanned = plan_metric_int(scan, "bytes_scanned", case.label)
        require(
            file_groups <= max_file_groups,
            f"{case.label} planned {scan['file_groups']} file groups, expected <= {max_file_groups}",
        )
        require(
            bytes_scanned <= max_bytes_scanned,
            f"{case.label} scanned {scan['bytes_scanned']} bytes, expected <= {max_bytes_scanned}",
        )

        print(
            "qps_scan",
            f"label={case.label}",
            f"expected_count={expected_count}",
            f"output_rows={scan['output_rows']}",
            f"bytes_scanned={scan['bytes_scanned']}",
            f"file_groups={scan['file_groups']}",
            f"row_groups_pruned_statistics={scan['row_groups_pruned_statistics']}",
            f"files_ranges_pruned_statistics={scan['files_ranges_pruned_statistics']}",
            f"hidden_bbox={scan['hidden_bbox']}",
            f"parquet_predicate={scan['parquet_predicate']}",
        )
        plans.append(QueryPlan(case, sql, expected_count))
    return plans


def linkerd_metric_line(line: str, dst_service: str) -> bool:
    if not line or line.startswith("#"):
        return False
    if 'direction="outbound"' not in line or 'tls="true"' not in line:
        return False
    return f'dst_service="{dst_service}"' in line or f'authority="{dst_service}.' in line


def metric_value(line: str) -> float:
    parts = line.split()
    require(len(parts) >= 2, f"invalid Prometheus metric line: {line!r}")
    return float(parts[-1])


def linkerd_snapshot(metrics_url: str, dst_service: str) -> LinkerdSnapshot:
    with urllib.request.urlopen(metrics_url, timeout=10) as response:
        text = response.read().decode("utf-8", errors="replace")

    open_total = 0.0
    read_bytes = 0.0
    write_bytes = 0.0
    dst_pods: set[str] = set()
    for line in text.splitlines():
        if not linkerd_metric_line(line, dst_service):
            continue
        if line.startswith("tcp_open_total{"):
            open_total += metric_value(line)
        elif line.startswith("tcp_read_bytes_total{"):
            read_bytes += metric_value(line)
        elif line.startswith("tcp_write_bytes_total{"):
            write_bytes += metric_value(line)

        match = re.search(r'dst_pod="([^"]+)"', line)
        if match:
            dst_pods.add(match.group(1))

    return LinkerdSnapshot(open_total, read_bytes, write_bytes, frozenset(dst_pods))


def print_linkerd_delta(before: LinkerdSnapshot, after: LinkerdSnapshot) -> tuple[float, float, float]:
    open_delta = after.open_total - before.open_total
    read_delta = after.read_bytes - before.read_bytes
    write_delta = after.write_bytes - before.write_bytes
    print(
        "qps_linkerd",
        f"tcp_open_delta={open_delta:.0f}",
        f"tcp_read_bytes_delta={read_delta:.0f}",
        f"tcp_write_bytes_delta={write_delta:.0f}",
        f"dst_pods={','.join(sorted(after.dst_pods))}",
    )
    return open_delta, read_delta, write_delta


def print_label_summaries(samples: list[read_probe.Sample]) -> None:
    labels = sorted({sample.label for sample in samples})
    for label in labels:
        label_samples = [sample for sample in samples if sample.label == label]
        summary = read_probe.latency_summary(label_samples)
        print(
            "qps_case_result",
            f"label={label}",
            f"queries={len(label_samples)}",
            f"avg_ms={summary['avg_ms']:.2f}",
            f"p50_ms={summary['p50_ms']:.2f}",
            f"p95_ms={summary['p95_ms']:.2f}",
            f"p99_ms={summary['p99_ms']:.2f}",
        )


def run_probe(
    table: str,
    factor: int,
    workers: int,
    total_queries: int,
    min_instances: int,
    min_qps: float,
    metrics_url: str | None,
    require_linkerd: bool,
    linkerd_dst_service: str,
    seeded_rows: int | str,
    max_file_groups: int,
    max_bytes_scanned: int,
) -> int:
    require(total_queries >= min_instances, "QPS_QUERIES must cover QPS_MIN_INSTANCES")

    plans = prepare_query_plans(table, factor, max_file_groups, max_bytes_scanned)
    before_metrics = linkerd_snapshot(metrics_url, linkerd_dst_service) if metrics_url else None

    print(
        "qps_config",
        f"table=public.{table}",
        f"seeded_rows={seeded_rows}",
        f"factor={factor}",
        f"workers={workers}",
        f"queries={total_queries}",
        f"cases={','.join(plan.case.label for plan in plans)}",
        f"min_instances={min_instances}",
        f"min_qps={min_qps:.2f}",
        f"max_file_groups={max_file_groups}",
        f"max_bytes_scanned={max_bytes_scanned}",
        f"linkerd_metrics={bool(metrics_url)}",
    )

    assignments = [total_queries // workers] * workers
    for idx in range(total_queries % workers):
        assignments[idx] += 1

    started = time.perf_counter()
    results: list[WorkerResult] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [
            executor.submit(run_worker, worker_id, assigned, plans)
            for worker_id, assigned in enumerate(assignments)
            if assigned > 0
        ]
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())
    elapsed_s = time.perf_counter() - started

    all_samples = [sample for result in results for sample in result.samples]
    instance_counts = Counter(result.instance_id for result in results)
    require_equal(len(all_samples), total_queries, "completed QPS query count")
    require(
        len(instance_counts) >= min_instances,
        f"expected >= {min_instances} backend instances, got {dict(instance_counts)!r}",
    )

    if metrics_url and before_metrics:
        after_metrics = linkerd_snapshot(metrics_url, linkerd_dst_service)
        open_delta, read_delta, write_delta = print_linkerd_delta(before_metrics, after_metrics)
        if require_linkerd:
            require(open_delta > 0, "Linkerd metrics did not record outbound TLS TCP opens")
            require(read_delta > 0, "Linkerd metrics did not record outbound TLS TCP read bytes")
            require(write_delta > 0, "Linkerd metrics did not record outbound TLS TCP write bytes")
            require(
                len(after_metrics.dst_pods) >= min_instances,
                f"Linkerd metrics saw fewer than {min_instances} destination pods: {after_metrics.dst_pods!r}",
            )

    summary = read_probe.latency_summary(all_samples)
    qps = total_queries / elapsed_s if elapsed_s else 0.0
    require(qps >= min_qps, f"qps {qps:.2f} below minimum {min_qps:.2f}")

    instances = sorted(instance_counts)
    print("qps_instances", ",".join(instances))
    print("qps_instance_counts", ",".join(f"{name}:{instance_counts[name]}" for name in instances))
    print_label_summaries(all_samples)
    print(
        "qps_result",
        f"queries={total_queries}",
        f"workers={workers}",
        f"elapsed_ms={elapsed_s * 1000.0:.2f}",
        f"qps={qps:.2f}",
        f"avg_ms={summary['avg_ms']:.2f}",
        f"p50_ms={summary['p50_ms']:.2f}",
        f"p95_ms={summary['p95_ms']:.2f}",
        f"p99_ms={summary['p99_ms']:.2f}",
    )
    print("qps_ok", True)
    return 0


def main() -> int:
    mode = os.environ.get("QPS_MODE", "seed-probe")
    factor = int_env("QPS_FACTOR", 50)
    workers = int_env("QPS_WORKERS", 16)
    total_queries = int_env("QPS_QUERIES", 240)
    min_instances = int_env("QPS_MIN_INSTANCES", 2)
    min_qps = float_env("QPS_MIN_QPS", 1.0)
    compact = bool_env("QPS_COMPACT", True)
    metrics_url = os.environ.get("QPS_LINKERD_METRICS_URL")
    require_linkerd = bool_env("QPS_REQUIRE_LINKERD", False)
    linkerd_dst_service = os.environ.get("QPS_LINKERD_DST_SERVICE", "lake")
    max_file_groups = int_env("QPS_MAX_FILE_GROUPS", 1)
    max_bytes_scanned = int_env("QPS_MAX_BYTES_SCANNED", 1024)
    table = os.environ.get("QPS_TABLE") or table_name("qps_aerial")

    rows: int | str = "preseeded"
    if mode in ("seed", "seed-probe"):
        rows = create_and_seed(table, factor, compact=compact)
        print("qps_seed", f"table=public.{table}", f"rows={rows}", f"factor={factor}", f"compact={compact}")
        if mode == "seed":
            print("qps_seed_ok", True)
            return 0

    require(mode in ("probe", "seed-probe"), f"unsupported QPS_MODE {mode!r}")
    return run_probe(
        table,
        factor,
        workers,
        total_queries,
        min_instances,
        min_qps,
        metrics_url,
        require_linkerd,
        linkerd_dst_service,
        rows,
        max_file_groups,
        max_bytes_scanned,
    )


if __name__ == "__main__":
    sys.exit(main())
