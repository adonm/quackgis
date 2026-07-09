#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail closed on QuackGIS metrics artifacts that exceed explicit budgets.

The dashboard renderer is intentionally descriptive. This companion script is a
cheap release/local gate: it reads one or more `metrics.json` artifacts, fails on
failed probe statuses, and enforces any numeric `*_budget` values emitted by the
probes.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


BUDGET_PAIRS = [
    ("max_scan_bytes", "max_scan_bytes_budget"),
    ("bytes_scanned", "bytes_scanned_budget"),
    ("max_file_groups", "max_file_groups_budget"),
    ("file_groups", "file_groups_budget"),
    ("row_groups", "row_groups_budget"),
    ("candidate_rows", "candidate_rows_budget"),
    ("candidate_groups", "candidate_groups_budget"),
    ("p95_ms", "p95_ms_budget"),
    ("p99_ms", "p99_ms_budget"),
    ("failed_commits", "failed_commits_budget"),
    ("retry_attempts", "retry_attempts_budget"),
    ("native_delete_files", "native_delete_files_budget"),
    ("native_update_appended_files", "native_update_appended_files_budget"),
    ("native_compact_appended_files", "native_compact_appended_files_budget"),
    ("native_mutation_aborts", "native_mutation_aborts_budget"),
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


def number(value: Any) -> float | None:
    if isinstance(value, bool) or value is None or value == "":
        return None
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        try:
            return float(value)
        except ValueError:
            return None
    return None


def check_metrics(
    path: Path,
    data: dict[str, Any],
    *,
    allow_not_run: bool,
) -> tuple[list[str], int, set[str]]:
    errors: list[str] = []
    budget_assertions = 0
    seen_checks: set[str] = set()
    checks = data.get("checks", {})
    if not isinstance(checks, dict):
        return ([f"{path}: missing object checks"], 0, seen_checks)

    for check_id, check_data in sorted(checks.items()):
        if not isinstance(check_data, dict):
            errors.append(f"{path}: checks.{check_id} must be an object")
            continue
        seen_checks.add(str(check_id))
        status = check_data.get("status")
        if status == "fail":
            errors.append(f"{path}: checks.{check_id} status=fail")
        elif status == "not run" and not allow_not_run:
            errors.append(f"{path}: checks.{check_id} status=not run")
        elif status not in {"pass", "not run"}:
            errors.append(f"{path}: checks.{check_id} has unknown status {status!r}")

        metrics = check_data.get("metrics", {}) or {}
        if not isinstance(metrics, dict):
            errors.append(f"{path}: checks.{check_id}.metrics must be an object")
            continue
        for metric_key, budget_key in BUDGET_PAIRS:
            metric = number(metrics.get(metric_key))
            budget = number(metrics.get(budget_key))
            if budget is None:
                continue
            budget_assertions += 1
            if metric is None:
                errors.append(
                    f"{path}: checks.{check_id}.{budget_key} is set but {metric_key} is missing/non-numeric"
                )
                continue
            if metric > budget:
                errors.append(
                    f"{path}: checks.{check_id}.{metric_key}={metric:g} exceeds {budget_key}={budget:g}"
                )
    return errors, budget_assertions, seen_checks


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", type=Path, help="metrics.json files or directories")
    parser.add_argument(
        "--allow-not-run",
        action="store_true",
        help="do not fail checks that are absent/not run in partial local reports",
    )
    parser.add_argument(
        "--require-budgeted",
        action="store_true",
        help="fail if no explicit metric budget was asserted",
    )
    parser.add_argument(
        "--require-check",
        action="append",
        default=[],
        help="require a named check id to appear with status=pass; may be repeated",
    )
    args = parser.parse_args(argv)

    metrics_paths = discover_metrics(args.paths)
    if not metrics_paths:
        print("no metrics.json files found", file=sys.stderr)
        return 1

    errors: list[str] = []
    budget_assertions = 0
    seen_checks: set[str] = set()
    passed_checks: set[str] = set()
    for path in metrics_paths:
        data = json.loads(path.read_text(encoding="utf-8"))
        artifact_errors, artifact_budgets, artifact_checks = check_metrics(
            path, data, allow_not_run=args.allow_not_run
        )
        errors.extend(artifact_errors)
        budget_assertions += artifact_budgets
        seen_checks.update(artifact_checks)
        for check_id, check_data in (data.get("checks", {}) or {}).items():
            if isinstance(check_data, dict) and check_data.get("status") == "pass":
                passed_checks.add(str(check_id))

    for required in args.require_check:
        if required not in seen_checks:
            errors.append(f"required check {required!r} was not present in any artifact")
        elif required not in passed_checks:
            errors.append(f"required check {required!r} did not pass")

    if args.require_budgeted and budget_assertions == 0:
        errors.append("no explicit metric budgets were asserted")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    print(
        "metrics_budget_check_ok "
        f"artifacts={len(metrics_paths)} checks={len(seen_checks)} budget_assertions={budget_assertions}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
