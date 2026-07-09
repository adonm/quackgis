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


METRIC_COLUMNS = [
    "qps",
    "p95_ms",
    "queries",
    "workers",
    "seeded_rows",
    "factor",
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
    "shared_rows",
    "failed_commits",
    "retry_attempts",
    "conflict_observed",
    "qgis_points_feature_count",
    "qgis_lines_feature_count",
    "qgis_multipolygons_feature_count",
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
    for check, check_data in sorted(data.get("checks", {}).items()):
        metrics = check_data.get("metrics", {}) or {}
        row: dict[str, str | int | float | bool] = {
            "source": str(path),
            "report_kind": row_value(run.get("report_kind")),
            "storage_recipe": row_value(run.get("storage_recipe")),
            "github_workflow": row_value(run.get("github_workflow")),
            "github_run_id": row_value(run.get("github_run_id")),
            "github_run_attempt": row_value(run.get("github_run_attempt")),
            "github_sha": row_value(run.get("github_sha")),
            "github_ref_name": row_value(run.get("github_ref_name")),
            "check": check,
            "label": row_value(check_data.get("label")),
            "status": row_value(check_data.get("status")),
            "summary_passed": row_value(summary.get("passed")),
            "summary_failed": row_value(summary.get("failed")),
            "summary_not_run": row_value(summary.get("not_run")),
        }
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
        "max_scan_bytes",
        "bytes_scanned",
        "shared_rows",
        "failed_commits",
    ]
    lines = ["| " + " | ".join(columns) + " |", "|" + "---|" * len(columns)]
    for row in rows:
        lines.append("| " + " | ".join(str(row.get(column, "")) for column in columns) + " |")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", type=Path, help="metrics.json file or artifact directory")
    parser.add_argument("--format", choices=("csv", "json", "markdown"), default="csv")
    args = parser.parse_args(argv)

    metric_paths = discover_metrics(args.paths)
    rows = [row for path in metric_paths for row in rows_for_metrics(path)]

    if args.format == "csv":
        render_csv(rows)
    elif args.format == "json":
        print(json.dumps(rows, indent=2, sort_keys=True))
    else:
        print(render_markdown(rows), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
