#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Classify and execute the maintained PostGIS subset against pinned DuckDB."""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGRESS = ROOT / "crates/quackgis-server/tests/postgis_regress.rs"
DEFAULT_LEDGER = ROOT / "tests/duckdb_spatial_compat.json"
CASE_RE = re.compile(
    r'Case\s*\{\s*name:\s*"(?P<name>[^"]+)",\s*'
    r'sql:\s*"(?P<sql>[^"]+)",\s*expected:\s*"(?P<expected>[^"]+)"',
    re.DOTALL,
)
DISPOSITIONS = {
    "native_duckdb",
    "sql_rewrite",
    "quackgis_macro",
    "rust_edge",
    "extension_candidate",
    "unsupported",
}
EXECUTABLE = {"native_duckdb", "sql_rewrite", "quackgis_macro"}


def load_regress_cases(path: Path = REGRESS) -> dict[str, dict[str, str]]:
    cases = {}
    for match in CASE_RE.finditer(path.read_text(encoding="utf-8")):
        case = match.groupdict()
        cases[case["name"]] = case
    if not cases:
        raise ValueError(f"no PostGIS cases parsed from {path}")
    return cases


def load_ledger(path: Path, regress: dict[str, dict[str, str]]) -> list[dict[str, str]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    entries = document.get("cases")
    if document.get("version") != 1 or not isinstance(entries, list):
        raise ValueError("DuckDB spatial ledger must have version=1 and a cases array")
    names = [entry.get("name") for entry in entries]
    duplicates = sorted(name for name, count in Counter(names).items() if count > 1)
    missing = sorted(set(regress) - set(names))
    extra = sorted(set(names) - set(regress))
    if duplicates or missing or extra:
        raise ValueError(
            f"ledger coverage mismatch duplicates={duplicates} missing={missing} extra={extra}"
        )
    for entry in entries:
        disposition = entry.get("disposition")
        if disposition not in DISPOSITIONS:
            raise ValueError(f"invalid disposition for {entry.get('name')}: {disposition}")
        if disposition in {"sql_rewrite", "quackgis_macro"} and not entry.get("duckdb_sql"):
            raise ValueError(f"{entry['name']} requires duckdb_sql for {disposition}")
    return entries


def normalize(value: str) -> str:
    value = value.strip()
    if re.match(
        r"^(SRID=\d+;)?(POINT|LINESTRING|POLYGON|MULTI|GEOMETRYCOLLECTION)",
        value,
        re.IGNORECASE,
    ):
        return re.sub(r"\s+", "", value).upper()
    return value.lower() if value.lower() in {"true", "false"} else value


def run_sql(binary: str, sql: str, env: dict[str, str]) -> tuple[str, str]:
    result = subprocess.run(
        [binary, "-csv", "-noheader", ":memory:", "-c", f"LOAD spatial; {sql}"],
        text=True,
        capture_output=True,
        check=False,
        env=env,
    )
    if result.returncode != 0:
        return "fail", result.stderr.strip() or result.stdout.strip()
    rows = list(csv.reader(result.stdout.splitlines()))
    if len(rows) != 1 or len(rows[0]) != 1:
        return "fail", f"expected one scalar row, got {rows!r}"
    return "pass", rows[0][0]


def git_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], text=True, capture_output=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def evaluate(
    binary: str,
    entries: list[dict[str, str]],
    regress: dict[str, dict[str, str]],
) -> dict[str, Any]:
    env = dict(os.environ)
    results = []
    for entry in entries:
        case = regress[entry["name"]]
        disposition = entry["disposition"]
        result: dict[str, Any] = {
            "name": entry["name"],
            "disposition": disposition,
            "status": "classified",
        }
        if disposition in EXECUTABLE:
            sql = entry.get("duckdb_sql", case["sql"])
            status, actual = run_sql(binary, sql, env)
            expected = entry.get("expected", case["expected"])
            if status == "pass" and normalize(actual) != normalize(expected):
                status = "fail"
                actual = f"expected {expected!r}, got {actual!r}"
            result.update(status=status, sql=sql, expected=expected, actual=actual)
        results.append(result)
    counts = Counter(result["disposition"] for result in results)
    failures = sum(result["status"] == "fail" for result in results)
    return {
        "source_sha": git_sha(),
        "claim": "duckdb_spatial_compatibility_classification",
        "case_count": len(results),
        "counts": dict(sorted(counts.items())),
        "failures": failures,
        "results": results,
    }


def render_markdown(summary: dict[str, Any]) -> str:
    lines = [
        "# DuckDB spatial compatibility classification",
        "",
        f"Status: `{'pass' if summary['failures'] == 0 else 'fail'}`",
        f"Source SHA: `{summary['source_sha']}`",
        f"Maintained cases: {summary['case_count']}",
        f"Executable failures: {summary['failures']}",
        "",
        "| Disposition | Count |",
        "|---|---:|",
    ]
    for disposition, count in summary["counts"].items():
        lines.append(f"| `{disposition}` | {count} |")
    lines.extend(["", "| Case | Disposition | Status |", "|---|---|---|"])
    for result in summary["results"]:
        lines.append(
            f"| `{result['name']}` | `{result['disposition']}` | {result['status']} |"
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb-bin", default="duckdb")
    parser.add_argument("--ledger", type=Path, default=DEFAULT_LEDGER)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        regress = load_regress_cases()
        entries = load_ledger(args.ledger, regress)
        summary = evaluate(args.duckdb_bin, entries, regress)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render_markdown(summary), encoding="utf-8")
        args.manifest.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"duckdb spatial compatibility probe failed: {error}", file=sys.stderr)
        return 2
    if summary["failures"]:
        print(
            f"duckdb_spatial_compat_failed failures={summary['failures']} out={args.out}",
            file=sys.stderr,
        )
        return 1
    print(f"duckdb_spatial_compat_ok cases={summary['case_count']} out={args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
