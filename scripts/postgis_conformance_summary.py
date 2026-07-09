#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Summarize QuackGIS PostGIS conformance fixture coverage.

The summary is intentionally static: it counts asserted fixture rows so docs and
release notes can describe the current conformance evidence without manually
recounting SQL files.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PGWIRE_REGRESS = ROOT / "crates/quackgis-server/tests/postgis_regress.rs"
PORT_CASES = ROOT / "tests/postgis_port/cases"
UPSTREAM_CURATED = ROOT / "tests/upstream_curated"

ASSERTION_RE = re.compile(r"THEN\s+'(?P<status>PASS|DELTA|SKIP)\b", re.IGNORECASE)


def count_pgwire_cases() -> int:
    text = PGWIRE_REGRESS.read_text(encoding="utf-8")
    return len(re.findall(r"(?m)^\s*Case\s*\{", text))


def summarize_sql_dir(path: Path) -> list[dict[str, Any]]:
    rows = []
    for sql_file in sorted(path.glob("*.sql")):
        text = sql_file.read_text(encoding="utf-8")
        counts = {"PASS": 0, "DELTA": 0, "SKIP": 0}
        for match in ASSERTION_RE.finditer(text):
            counts[match.group("status").upper()] += 1
        rows.append(
            {
                "file": str(sql_file.relative_to(ROOT)),
                "pass": counts["PASS"],
                "delta": counts["DELTA"],
                "skip": counts["SKIP"],
                "total": sum(counts.values()),
            }
        )
    return rows


def build_summary() -> dict[str, Any]:
    port_cases = summarize_sql_dir(PORT_CASES)
    upstream = summarize_sql_dir(UPSTREAM_CURATED)
    pgwire_cases = count_pgwire_cases()
    return {
        "pgwire_regress_cases": pgwire_cases,
        "sql_portability_cases": {
            "postgis_port": port_cases,
            "upstream_curated": upstream,
            "postgis_port_total": sum(row["total"] for row in port_cases),
            "upstream_curated_total": sum(row["total"] for row in upstream),
            "total": sum(row["total"] for row in port_cases + upstream),
        },
    }


def render_markdown(summary: dict[str, Any]) -> str:
    portability = summary["sql_portability_cases"]
    lines = [
        "# PostGIS conformance fixture summary",
        "",
        f"- Pgwire claimed subset cases: **{summary['pgwire_regress_cases']}**",
        f"- SQL portability assertions: **{portability['total']}**",
        f"  - `tests/postgis_port/cases`: **{portability['postgis_port_total']}**",
        f"  - `tests/upstream_curated`: **{portability['upstream_curated_total']}**",
        "",
        "## SQL fixture files",
        "",
        "| File | PASS | DELTA | SKIP | Total |",
        "|---|---:|---:|---:|---:|",
    ]
    for group in ("postgis_port", "upstream_curated"):
        for row in portability[group]:
            lines.append(
                f"| `{row['file']}` | {row['pass']} | {row['delta']} | {row['skip']} | {row['total']} |"
            )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--format", choices=("markdown", "json"), default="markdown")
    args = parser.parse_args()

    summary = build_summary()
    if args.format == "json":
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(render_markdown(summary), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
