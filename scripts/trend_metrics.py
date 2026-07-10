#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Summarize QuackGIS compatibility/storage metrics artifacts.

Input paths may be individual `metrics.json` files or directories containing
one or more report artifacts. Output is intentionally flat so scheduled Alpha
and compatibility runs can be compared with ordinary CSV/JSON tooling.
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from benchmark_profile_check import parse_rfc3339  # noqa: E402

METRIC_COLUMNS = [
    "qps",
    "p95_ms",
    "p99_ms",
    "queries",
    "workers",
    "seeded_rows",
    "factor",
    "benchmark_profile",
    "dataset_rows",
    "storage_profile",
    "hardware_profile",
    "memory_bytes",
    "free_disk_bytes",
    "object_bytes",
    "elapsed_seconds",
    "bytes_scanned",
    "max_scan_bytes",
    "max_scan_bytes_budget",
    "max_file_groups",
    "max_file_groups_budget",
    "bytes_scanned_budget",
    "file_groups",
    "groups",
    "candidate_groups",
    "candidate_rows",
    "feature_count",
    "reflected_columns",
    "bbox_count",
    "tile_bytes",
    "shared_rows",
    "failed_commits",
    "retry_attempts",
    "conflict_observed",
    "native_delete_files",
    "native_delete_snapshots",
    "native_update_delete_files",
    "native_update_appended_files",
    "native_compact_delete_files",
    "native_compact_appended_files",
    "native_compact_retired_files",
    "native_mutation_aborts",
    "catalog_roundtrips",
    "catalog_roundtrips_budget",
    "catalog_read_roundtrips",
    "catalog_read_roundtrips_budget",
    "warm_public_queries",
    "catalog_read_provider_calls",
    "catalog_read_provider_calls_budget",
    "warm_public_catalog_read_provider_calls",
    "warm_public_catalog_read_provider_calls_budget",
    "catalog_read_provider_calls_per_query_max",
    "catalog_read_provider_calls_per_query_max_budget",
    "cold_public_catalog_read_provider_calls",
    "cold_public_catalog_read_provider_calls_budget",
    "direct_internal_catalog_read_provider_calls",
    "direct_internal_catalog_read_provider_calls_budget",
    "catalog_provider_call_scope",
    "catalog_write_roundtrips",
    "catalog_write_roundtrips_budget",
    "catalog_refreshes",
    "catalog_refreshes_budget",
    "qgis_points_feature_count",
    "qgis_lines_feature_count",
    "qgis_multipolygons_feature_count",
    "mvt_points_tile_bytes",
    "mvt_lines_tile_bytes",
    "mvt_multipolygons_tile_bytes",
    "postgis_passed",
    "postgis_total",
    "postgis_total_min",
    "postgis_pass_rate",
    "postgis_pass_rate_min",
]

BASE_COLUMNS = [
    "source",
    "report_kind",
    "storage_recipe",
    "github_workflow",
    "github_run_id",
    "github_run_attempt",
    "github_sha",
    "github_ref_name",
    "run_started_at",
    "check",
    "label",
    "status",
    "summary_passed",
    "summary_failed",
    "summary_not_run",
]


def discover_metrics(paths: list[Path]) -> list[Path]:
    metrics: list[Path] = []
    for path in paths:
        if path.is_file():
            metrics.append(path)
        elif path.is_dir():
            metrics.extend(sorted(path.rglob("metrics.json")))
        else:
            raise FileNotFoundError(path)
    return sorted(dict.fromkeys(metrics))


def row_value(value: Any) -> str | int | float | bool:
    if value is None:
        return ""
    if isinstance(value, (str, int, float, bool)):
        return value
    return json.dumps(value, sort_keys=True)


def rows_for_metrics(path: Path) -> list[dict[str, str | int | float | bool]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    run = data.get("run", {})
    summary = data.get("summary", {})
    rows = []

    def base_row(check: str, label: Any, status: Any) -> dict[str, str | int | float | bool]:
        return {
            "source": str(path),
            "report_kind": row_value(run.get("report_kind")),
            "storage_recipe": row_value(run.get("storage_recipe")),
            "github_workflow": row_value(run.get("github_workflow")),
            "github_run_id": row_value(run.get("github_run_id")),
            "github_run_attempt": row_value(run.get("github_run_attempt")),
            "github_sha": row_value(run.get("github_sha")),
            "github_ref_name": row_value(run.get("github_ref_name")),
            "run_started_at": row_value(run.get("run_started_at")),
            "check": check,
            "label": row_value(label),
            "status": row_value(status),
            "summary_passed": row_value(summary.get("passed")),
            "summary_failed": row_value(summary.get("failed")),
            "summary_not_run": row_value(summary.get("not_run")),
        }

    postgis_subset = data.get("postgis_regress_subset")
    if isinstance(postgis_subset, dict):
        passed = postgis_subset.get("passed")
        total = postgis_subset.get("total")
        status = "pass" if passed == total else "fail"
        row = base_row("postgis_regress", "PostGIS regress subset", status)
        row["postgis_passed"] = row_value(passed)
        row["postgis_total"] = row_value(total)
        row["postgis_pass_rate"] = row_value(postgis_subset.get("pass_rate"))
        rows.append(row)

    for check, check_data in sorted(data.get("checks", {}).items()):
        metrics = check_data.get("metrics", {}) or {}
        row = base_row(check, check_data.get("label"), check_data.get("status"))
        for column in METRIC_COLUMNS:
            row[column] = row_value(metrics.get(column))
        rows.append(row)
    return rows


def render_csv(rows: list[dict[str, Any]]) -> str:
    columns = BASE_COLUMNS + METRIC_COLUMNS
    out = sys.stdout
    writer = csv.DictWriter(out, fieldnames=columns, extrasaction="ignore")
    writer.writeheader()
    writer.writerows(rows)
    return ""


def render_markdown(rows: list[dict[str, Any]]) -> str:
    columns = [
        "report_kind",
        "storage_recipe",
        "github_run_id",
        "github_sha",
        "check",
        "status",
        "qps",
        "p95_ms",
        "p99_ms",
        "max_scan_bytes",
        "bytes_scanned",
        "shared_rows",
        "failed_commits",
        "native_delete_files",
        "native_update_appended_files",
        "native_compact_appended_files",
        "native_compact_retired_files",
        "native_mutation_aborts",
        "postgis_pass_rate",
        "postgis_total_min",
        "postgis_pass_rate_min",
    ]
    lines = ["| " + " | ".join(columns) + " |", "|" + "---|" * len(columns)]
    for row in rows:
        lines.append("| " + " | ".join(str(row.get(column, "")) for column in columns) + " |")
    return "\n".join(lines) + "\n"


def present(row: dict[str, Any], column: str) -> bool:
    value = row.get(column)
    return value is not None and value != ""


def metric_list(row: dict[str, Any], columns: list[str]) -> str:
    return ", ".join(f"{column}={row[column]}" for column in columns if present(row, column))


def run_label(row: dict[str, Any]) -> str:
    run_id = row.get("github_run_id")
    attempt = row.get("github_run_attempt")
    if run_id and attempt:
        return f"{run_id}.{attempt}"
    if run_id:
        return str(run_id)
    return str(row.get("source", ""))


def sort_value(value: Any) -> tuple[int, int, str]:
    if value is None or value == "":
        return (0, 0, "")
    if isinstance(value, int) and not isinstance(value, bool):
        return (2, value, "")
    if isinstance(value, str) and value.isdigit():
        return (2, int(value), "")
    return (1, 0, str(value))


def timestamp_sort_value(value: Any) -> tuple[int, float, str]:
    if not isinstance(value, str):
        return (0, 0.0, "")
    try:
        parsed = parse_rfc3339(value)
    except ValueError:
        return (1, 0.0, value)
    if parsed.tzinfo is None:
        return (1, 0.0, value)
    return (2, parsed.timestamp(), value)


def row_sort_key(row: dict[str, Any]) -> tuple[Any, ...]:
    return (
        timestamp_sort_value(row.get("run_started_at")),
        sort_value(row.get("github_run_id")),
        sort_value(row.get("github_run_attempt")),
        sort_value(row.get("github_sha")),
        sort_value(row.get("source")),
        sort_value(row.get("check")),
    )


def latest_by_check(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    latest: dict[str, dict[str, Any]] = {}
    for row in sorted(rows, key=row_sort_key):
        latest[str(row.get("check", ""))] = row
    return [latest[key] for key in sorted(latest)]


def scan_budget(row: dict[str, Any]) -> str:
    values = []
    if present(row, "max_scan_bytes") or present(row, "max_scan_bytes_budget"):
        values.append(f"max_scan_bytes={row.get('max_scan_bytes', '')}/{row.get('max_scan_bytes_budget', '')}")
    if present(row, "bytes_scanned") or present(row, "bytes_scanned_budget"):
        values.append(f"bytes_scanned={row.get('bytes_scanned', '')}/{row.get('bytes_scanned_budget', '')}")
    if present(row, "max_file_groups") or present(row, "max_file_groups_budget"):
        values.append(f"max_file_groups={row.get('max_file_groups', '')}/{row.get('max_file_groups_budget', '')}")
    return ", ".join(values)


def catalog_budget(row: dict[str, Any]) -> str:
    values = []
    for metric in (
        "catalog_roundtrips",
        "catalog_read_roundtrips",
        "catalog_read_provider_calls",
        "warm_public_catalog_read_provider_calls",
        "catalog_read_provider_calls_per_query_max",
        "cold_public_catalog_read_provider_calls",
        "direct_internal_catalog_read_provider_calls",
        "catalog_write_roundtrips",
        "catalog_refreshes",
    ):
        budget = f"{metric}_budget"
        if present(row, metric) or present(row, budget):
            values.append(f"{metric}={row.get(metric, '')}/{row.get(budget, '')}")
    if present(row, "catalog_provider_call_scope"):
        values.append(f"provider_scope={row['catalog_provider_call_scope']}")
    return ", ".join(values)


def render_dashboard(rows: list[dict[str, Any]]) -> str:
    latest = latest_by_check(rows)
    sources = sorted({str(row.get("source", "")) for row in rows if row.get("source")})
    body = [
        "# QuackGIS metrics trend dashboard",
        "",
        f"Generated from **{len(rows)} check rows** across **{len(sources)} metrics artifact(s)**.",
        "",
        "This dashboard is intentionally plain Markdown so scheduled artifacts can be",
        "checked in for releases or attached unchanged to release evidence. It keeps the",
        "roadmap signals visible in one place: profile identity, QPS, p95/p99 latency,",
        "scan/catalog budgets, candidate narrowing, native DML/compaction/abort counters, writer conflicts, and",
        "PostGIS regress pass-rate when the artifact includes that log.",
        "",
        "## Latest row per check",
        "",
        "| Check | Status | Run | SHA | Profile | Rows | QPS | p95 ms | p99 ms | Scan budgets | Catalog budgets | Candidate rows | Mutation/conflict counters | PostGIS pass-rate |",
        "|---|---|---|---|---|---:|---:|---:|---:|---|---|---:|---|---:|",
    ]
    for row in latest:
        body.append(
            "| "
            + " | ".join(
                [
                    str(row.get("check", "")),
                    str(row.get("status", "")),
                    run_label(row),
                    str(row.get("github_sha", ""))[:12],
                    str(row.get("benchmark_profile", "")),
                    str(row.get("dataset_rows", "")),
                    str(row.get("qps", "")),
                    str(row.get("p95_ms", "")),
                    str(row.get("p99_ms", "")),
                    scan_budget(row),
                    catalog_budget(row),
                    str(row.get("candidate_rows", "")),
                    metric_list(
                        row,
                        [
                            "failed_commits",
                            "retry_attempts",
                            "native_delete_files",
                            "native_update_appended_files",
                            "native_compact_appended_files",
                            "native_compact_retired_files",
                        ],
                    ),
                    str(row.get("postgis_pass_rate", "")),
                ]
            )
            + " |"
        )

    body.extend(
        [
            "",
            "## Roadmap signal coverage",
            "",
            "| Signal | Checks that usually populate it | Latest values present |",
            "|---|---|---|",
            f"| QPS + p95/p99 latency | `read_probe`, `qps_probe`, `olap_probe` | {sum(1 for row in latest if present(row, 'qps') or present(row, 'p95_ms') or present(row, 'p99_ms'))} check(s) |",
            f"| Scan-byte and file-group budgets | `qps_probe`, `olap_probe` | {sum(1 for row in latest if scan_budget(row))} check(s) |",
            f"| Catalog provider-call, roundtrip, and refresh budgets | regional benchmark checks | {sum(1 for row in latest if catalog_budget(row))} check(s) |",
            f"| OLAP candidate narrowing | `olap_probe` | {sum(1 for row in latest if present(row, 'candidate_rows') or present(row, 'candidate_groups'))} check(s) |",
            f"| Writer conflict/retry | `write_verify` | {sum(1 for row in latest if present(row, 'failed_commits') or present(row, 'retry_attempts'))} check(s) |",
            f"| Native DML/compaction counters | `external_lake_probe` | {sum(1 for row in latest if present(row, 'native_delete_files') or present(row, 'native_compact_appended_files'))} check(s) |",
            f"| Client real-data counts | `osm_postgis_parity` | {sum(1 for row in latest if present(row, 'qgis_points_feature_count') or present(row, 'qgis_lines_feature_count') or present(row, 'qgis_multipolygons_feature_count') or present(row, 'mvt_points_tile_bytes'))} check(s) |",
            f"| API/client profile counts | `api_client_probe` | {sum(1 for row in latest if present(row, 'feature_count') or present(row, 'reflected_columns') or present(row, 'tile_bytes'))} check(s) |",
            f"| PostGIS pass-rate | `postgis_regress` | {sum(1 for row in latest if present(row, 'postgis_pass_rate'))} check(s) |",
            "",
        ]
    )
    return "\n".join(body)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", type=Path, help="metrics.json file or artifact directory")
    parser.add_argument("--format", choices=("csv", "json", "markdown", "dashboard"), default="csv")
    args = parser.parse_args(argv)

    metric_paths = discover_metrics(args.paths)
    rows = [row for path in metric_paths for row in rows_for_metrics(path)]

    if args.format == "csv":
        render_csv(rows)
    elif args.format == "json":
        print(json.dumps(rows, indent=2, sort_keys=True))
    elif args.format == "markdown":
        print(render_markdown(rows), end="")
    else:
        print(render_dashboard(rows), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
