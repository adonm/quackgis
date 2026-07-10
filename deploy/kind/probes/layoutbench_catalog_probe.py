# SPDX-License-Identifier: Apache-2.0
"""Bounded Kind runner for LayoutBench catalog provider-call evidence.

The probe has two explicit modes:

* ``LAYOUTBENCH_MODE=seed`` streams the exact 100M regional profile into the
  already deployed Kind lake service. This mode refuses to run unless the caller
  acknowledges the scale with ``LAYOUTBENCH_ALLOW_EXACT_R100M=true``.
* ``LAYOUTBENCH_MODE=measure`` assumes the exact tables already exist, runs the
  three committed catalog-metering phases through one lake pod, and emits
  ``layoutbench_catalog`` records consumed by ``scripts/layoutbench_catalog_report.py``.
"""

from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import sys
import time
import urllib.request
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Iterable

import read_probe
from probe_common import pg_connect, quote_ident, require, require_equal


EXPECTED_PROFILE_ID = "layoutbench-regional-r100m-v1"
EXPECTED_ROWS = 100_000_000
DEFAULT_PROFILE_PATH = "/opt/quackgis-benchmarks/layoutbench-regional-r100m-v1.json"
METRIC_PROVIDER_CALLS = "quackgis_catalog_read_provider_calls_total"
METRIC_CATALOG_REFRESHES = "quackgis_catalog_refresh_total"


@dataclass(frozen=True)
class TableProfile:
    table_id: str
    rows: int
    copy_batch_rows: int
    expected_batches: int


@dataclass(frozen=True)
class BenchmarkProfile:
    profile_id: str
    target_rows: int
    storage_profile: str
    row_group_rows: int
    warm_queries: int
    tables: tuple[TableProfile, ...]


@dataclass(frozen=True)
class MetricSnapshot:
    provider_calls: int
    refreshes: int


@dataclass(frozen=True)
class PhaseResult:
    phase: str
    queries: int
    provider_calls: int
    per_query_max: int
    refreshes: int
    counter_start: int
    counter_end: int
    count: int


class GeneratedCopyStream:
    """File-like COPY source for one table batch.

    It intentionally generates only one batch at a time so the exact 100M load
    path is bounded by the configured COPY batch size rather than by the whole
    profile.
    """

    def __init__(self, table_id: str, start_row: int, rows: int):
        self.table_id = table_id
        self.next_row = start_row
        self.stop_row = start_row + rows
        self.count = 0
        self._buffer = ""
        self.done = False

    def readable(self) -> bool:
        return True

    def read(self, size: int = -1) -> str:
        if size is None or size < 0:
            chunks = [self._buffer]
            self._buffer = ""
            while not self.done:
                line = self._next_line()
                if line is None:
                    break
                chunks.append(line)
            return "".join(chunks)

        while len(self._buffer) < size and not self.done:
            line = self._next_line()
            if line is None:
                break
            self._buffer += line
        out = self._buffer[:size]
        self._buffer = self._buffer[size:]
        return out

    def _next_line(self) -> str | None:
        if self.next_row >= self.stop_row:
            self.done = True
            return None
        row_id = self.next_row + 1
        rect = synthetic_rect(self.table_id, self.next_row)
        captured_minute = self.next_row % (5 * 366 * 24 * 60)
        quality = 1.0 + (self.next_row % 1000) / 100.0
        self.next_row += 1
        self.count += 1
        return (
            "\t".join(
                [
                    str(row_id),
                    str(captured_minute),
                    f"{quality:.2f}",
                    read_probe.copy_bytea_hex(read_probe.rect_wkb_hex(rect)),
                ]
            )
            + "\n"
        )


def bool_env(name: str, default: bool = False) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.lower() in ("1", "true", "yes", "on")


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def env_text(name: str, default: str) -> str:
    value = os.environ.get(name, default).strip()
    require(value, f"{name} must not be empty")
    return value


def table_prefix() -> str:
    return env_text("LAYOUTBENCH_TABLE_PREFIX", "layoutbench_regional_r100m")


def table_name(table_id: str) -> str:
    return f"{table_prefix()}_{table_id}"


def load_profile(path: str | Path) -> BenchmarkProfile:
    raw = json.loads(Path(path).read_text(encoding="utf-8"))
    require_equal(raw.get("profile_id"), EXPECTED_PROFILE_ID, "profile_id")
    require_equal(int(raw.get("target_rows", -1)), EXPECTED_ROWS, "target_rows")
    storage = raw.get("storage") or {}
    measurement = raw.get("measurement") or {}
    tables = tuple(
        TableProfile(
            table_id=str(table["id"]),
            rows=int(table["rows"]),
            copy_batch_rows=int(table["copy_batch_rows"]),
            expected_batches=int(table["expected_batches"]),
        )
        for table in raw.get("tables", [])
    )
    profile = BenchmarkProfile(
        profile_id=str(raw["profile_id"]),
        target_rows=int(raw["target_rows"]),
        storage_profile=str(storage["profile"]),
        row_group_rows=int(storage["row_group_rows"]),
        warm_queries=int(measurement["warm_public_selective_queries"]),
        tables=tables,
    )
    validate_profile(profile)
    return profile


def validate_profile(profile: BenchmarkProfile) -> None:
    require_equal(profile.profile_id, EXPECTED_PROFILE_ID, "profile id")
    require_equal(profile.target_rows, EXPECTED_ROWS, "target rows")
    require_equal(sum(table.rows for table in profile.tables), profile.target_rows, "table rows")
    require_equal(profile.warm_queries, 240, "warm query count")
    require(profile.tables, "profile must contain tables")
    for table in profile.tables:
        batches = copy_batches(table)
        require_equal(batches, table.expected_batches, f"{table.table_id} batch count")
        require(table.copy_batch_rows <= profile.row_group_rows, f"{table.table_id} batch must fit profile row group policy")


def copy_batches(table: TableProfile) -> int:
    return (table.rows + table.copy_batch_rows - 1) // table.copy_batch_rows


def seed_plan(profile: BenchmarkProfile) -> list[tuple[TableProfile, int, int]]:
    plan: list[tuple[TableProfile, int, int]] = []
    for table in profile.tables:
        start = 0
        while start < table.rows:
            rows = min(table.copy_batch_rows, table.rows - start)
            plan.append((table, start, rows))
            start += rows
    return plan


def require_seed_ack(profile: BenchmarkProfile) -> None:
    require(bool_env("LAYOUTBENCH_ALLOW_EXACT_R100M"), "set LAYOUTBENCH_ALLOW_EXACT_R100M=true to seed the exact 100M profile")
    max_rows = int_env("LAYOUTBENCH_MAX_ROWS", profile.target_rows)
    require(max_rows >= profile.target_rows, "LAYOUTBENCH_MAX_ROWS is below the profile target")


def synthetic_rect(table_id: str, row_index: int) -> read_probe.Rect:
    table_offset = {
        "aerial_frames": 0.0,
        "cad_objects": 125_000.0,
        "asset_footprints": 250_000.0,
    }.get(table_id, 0.0)
    x = table_offset + float((row_index * 37) % 400_000)
    y = float((row_index * 19) % 250_000)
    width = 18.0 + float(row_index % 13)
    height = 12.0 + float(row_index % 17)
    return read_probe.Rect(x, y, x + width, y + height)


def create_table(cur, table_id: str) -> None:
    name = quote_ident(table_name(table_id))
    cur.execute(f"DROP TABLE IF EXISTS public.{name}")
    cur.execute(
        f"CREATE TABLE public.{name} ("
        "id BIGINT, captured_minute INT, quality DOUBLE, geom BINARY)"
    )


def seed(profile: BenchmarkProfile) -> None:
    require_seed_ack(profile)
    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            for table in profile.tables:
                create_table(cur, table.table_id)
            for table, start, rows in seed_plan(profile):
                stream = GeneratedCopyStream(table.table_id, start, rows)
                started = time.perf_counter()
                cur.copy_expert(
                    f"COPY public.{quote_ident(table_name(table.table_id))} "
                    "(id, captured_minute, quality, geom) FROM STDIN",
                    stream,
                )
                require_equal(stream.count, rows, f"{table.table_id} streamed rows")
                print(
                    "layoutbench_seed_batch",
                    f"table={table.table_id}",
                    f"start={start}",
                    f"rows={rows}",
                    f"elapsed_seconds={time.perf_counter() - started:.3f}",
                )
    finally:
        conn.close()
    print(
        "layoutbench_seed_ok",
        f"profile_id={profile.profile_id}",
        f"target_rows={profile.target_rows}",
        f"batches={len(seed_plan(profile))}",
    )


def metrics_url() -> str:
    host = env_text("QUACKGIS_METRICS_HOST", os.environ.get("QUACKGIS_HOST", "lake.quackgis.svc.cluster.local"))
    port = int_env("QUACKGIS_METRICS_PORT", 9187)
    return f"http://{host}:{port}/metrics"


def scrape_metrics() -> MetricSnapshot:
    with urllib.request.urlopen(metrics_url(), timeout=10) as response:
        body = response.read().decode("utf-8")
    return MetricSnapshot(
        provider_calls=metric_int(body, METRIC_PROVIDER_CALLS),
        refreshes=metric_int(body, METRIC_CATALOG_REFRESHES),
    )


def metric_int(body: str, metric_name: str) -> int:
    pattern = re.compile(rf"^{re.escape(metric_name)}(?:\{{[^}}]*\}})?\s+([0-9]+(?:\.[0-9]+)?)$", re.MULTILINE)
    matches = pattern.findall(body)
    require(matches, f"missing metric {metric_name}")
    require_equal(len(matches), 1, f"metric {metric_name} series count")
    value = float(matches[0])
    require(value >= 0 and value.is_integer(), f"metric {metric_name} must be a non-negative integer")
    return int(value)


def public_query(table: str) -> str:
    case = read_probe.QueryCase(
        label="regional_catalog",
        geom_column="geom",
        envelope=read_probe.Rect(95.0, 95.0, 290.0, 185.0),
        extra_predicate="captured_minute BETWEEN 40 AND 170",
    )
    return read_probe.exact_sql(table, case)


def direct_query(table: str) -> str:
    case = read_probe.QueryCase(
        label="regional_catalog",
        geom_column="geom",
        envelope=read_probe.Rect(95.0, 95.0, 290.0, 185.0),
        extra_predicate="captured_minute BETWEEN 40 AND 170",
    )
    return read_probe.scan_evidence_sql(table, case)


def query_instance_id(cur) -> str:
    cur.execute("SELECT quackgis_instance_id()")
    value = str(cur.fetchone()[0])
    require(value, "quackgis_instance_id() returned an empty value")
    return value


def run_count(cur, sql: str) -> int:
    cur.execute(sql)
    return int(cur.fetchone()[0])


def measure_phase(cur, phase: str, sqls: Iterable[str]) -> PhaseResult:
    start = scrape_metrics()
    counts: list[int] = []
    per_query: list[int] = []
    for sql in sqls:
        before = scrape_metrics().provider_calls
        counts.append(run_count(cur, sql))
        after = scrape_metrics().provider_calls
        require(after >= before, f"{phase} provider-call counter reset inside query")
        per_query.append(after - before)
    end = scrape_metrics()
    require(end.provider_calls >= start.provider_calls, f"{phase} provider-call counter reset")
    require(end.refreshes >= start.refreshes, f"{phase} refresh counter reset")
    require(counts, f"{phase} did not execute queries")
    require(len(set(counts)) == 1, f"{phase} query counts changed during measurement: {counts[:5]!r}")
    return PhaseResult(
        phase=phase,
        queries=len(counts),
        provider_calls=end.provider_calls - start.provider_calls,
        per_query_max=max(per_query),
        refreshes=end.refreshes - start.refreshes,
        counter_start=start.provider_calls,
        counter_end=end.provider_calls,
        count=counts[0],
    )


def verify_exact_rows(cur, profile: BenchmarkProfile) -> None:
    total = 0
    for table in profile.tables:
        cur.execute(f"SELECT COUNT(*) FROM public.{quote_ident(table_name(table.table_id))}")
        count = int(cur.fetchone()[0])
        require_equal(count, table.rows, f"{table.table_id} row count")
        total += count
    require_equal(total, profile.target_rows, "total profile rows")


def run_metadata(profile: BenchmarkProfile, elapsed_seconds: float) -> dict[str, str]:
    source_sha = env_text("LAYOUTBENCH_SOURCE_SHA", os.environ.get("GITHUB_SHA", ""))
    require(re.fullmatch(r"[0-9a-f]{40}", source_sha), "LAYOUTBENCH_SOURCE_SHA/GITHUB_SHA must be a 40-character lowercase Git SHA")
    memory_bytes = int_env("LAYOUTBENCH_MEMORY_BYTES", memory_total_bytes())
    free_disk_bytes = int_env("LAYOUTBENCH_FREE_DISK_BYTES", shutil.disk_usage("/").free)
    object_bytes = int_env("LAYOUTBENCH_OBJECT_BYTES", 1)
    return {
        "source_sha": source_sha,
        "storage_profile": profile.storage_profile,
        "hardware_profile": env_text("LAYOUTBENCH_HARDWARE_PROFILE", "kind-local-v1"),
        "memory_bytes": str(memory_bytes),
        "free_disk_bytes": str(free_disk_bytes),
        "object_bytes": str(object_bytes),
        "elapsed_seconds": f"{elapsed_seconds:.3f}",
        "github_run_id": env_text("GITHUB_RUN_ID", os.environ.get("LAYOUTBENCH_RUN_ID", "1")),
        "github_run_attempt": env_text("GITHUB_RUN_ATTEMPT", os.environ.get("LAYOUTBENCH_RUN_ATTEMPT", "1")),
        "run_started_at": env_text(
            "LAYOUTBENCH_RUN_STARTED_AT",
            datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        ),
    }


def memory_total_bytes() -> int:
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except OSError:
        pass
    return 1


def phase_line(profile: BenchmarkProfile, metadata: dict[str, str], process_id: str, result: PhaseResult) -> str:
    fields = {
        "phase": result.phase,
        "profile_id": profile.profile_id,
        "target_rows": str(profile.target_rows),
        "warm_queries": str(profile.warm_queries),
        **metadata,
        "correctness": "pass",
        "server_process_id": process_id,
        "queries": str(result.queries),
        "catalog_read_provider_calls": str(result.provider_calls),
        "catalog_read_provider_calls_per_query_max": str(result.per_query_max),
        "catalog_refreshes": str(result.refreshes),
        "catalog_read_provider_calls_start": str(result.counter_start),
        "catalog_read_provider_calls_end": str(result.counter_end),
    }
    return "layoutbench_catalog " + " ".join(
        f"{key}={shlex.quote(value)}" for key, value in fields.items()
    )


def measure(profile: BenchmarkProfile) -> None:
    conn = pg_connect()
    conn.autocommit = True
    started = time.perf_counter()
    try:
        with conn.cursor() as cur:
            process_id = query_instance_id(cur)
            table = table_name(profile.tables[0].table_id)
            cold = measure_phase(cur, "cold_public", [public_query(table)])
            direct = measure_phase(cur, "direct_internal", [direct_query(table)])
            require_equal(direct.count, cold.count, "direct/public count parity")
            warm_sql = public_query(table)
            warm = measure_phase(cur, "warm_public", [warm_sql] * profile.warm_queries)
            require_equal(warm.count, cold.count, "warm/public count parity")
            verify_exact_rows(cur, profile)
            require_equal(query_instance_id(cur), process_id, "serving instance changed")
    finally:
        conn.close()
    metadata = run_metadata(profile, time.perf_counter() - started)
    for result in (cold, direct, warm):
        print(phase_line(profile, metadata, process_id, result))


def main() -> int:
    profile = load_profile(os.environ.get("LAYOUTBENCH_PROFILE", DEFAULT_PROFILE_PATH))
    mode = env_text("LAYOUTBENCH_MODE", "measure")
    if mode == "seed":
        seed(profile)
    elif mode == "measure":
        measure(profile)
    else:
        raise SystemExit(f"unknown LAYOUTBENCH_MODE={mode!r}; expected seed or measure")
    return 0


if __name__ == "__main__":
    sys.exit(main())
