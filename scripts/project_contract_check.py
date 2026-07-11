#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate maintained project claims, links, commands, and spatial counts."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")
JUST_COMMAND = re.compile(r"\bjust\s+([a-zA-Z0-9_-]+)")
CASE_NAME = re.compile(r'Case\s*\{\s*name:\s*"([^"]+)"', re.MULTILINE)
EXECUTABLE = {"native_duckdb", "sql_rewrite", "quackgis_macro"}
EXPECTED_COUNTS = {
    "native_duckdb": 31,
    "sql_rewrite": 5,
    "quackgis_macro": 6,
    "rust_edge": 10,
    "extension_candidate": 5,
}


def tracked_markdown() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "*.md"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [ROOT / line for line in result.stdout.splitlines() if line]


def just_recipes() -> set[str]:
    result = subprocess.run(
        ["just", "--summary"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return set(result.stdout.split())


def check_markdown(errors: list[str]) -> None:
    recipes = just_recipes()
    for path in tracked_markdown():
        text = path.read_text(encoding="utf-8")
        for raw_target in LINK.findall(text):
            target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
            if not target or target.startswith(("#", "http://", "https://", "mailto:")):
                continue
            local = unquote(target.split("#", 1)[0])
            if local and not (path.parent / local).exists():
                errors.append(f"{path.relative_to(ROOT)}: broken link {target!r}")
        for recipe in JUST_COMMAND.findall(text):
            if recipe.startswith("-"):
                continue
            if recipe not in recipes:
                errors.append(f"{path.relative_to(ROOT)}: unknown just recipe {recipe!r}")


def check_spatial_ledger(errors: list[str]) -> None:
    ledger_path = ROOT / "tests/duckdb_spatial_compat.json"
    fixture_path = ROOT / "tests/fixtures/postgis_curated_cases.rs"
    ledger = json.loads(ledger_path.read_text(encoding="utf-8"))["cases"]
    names = [case["name"] for case in ledger]
    fixture_names = CASE_NAME.findall(fixture_path.read_text(encoding="utf-8"))
    counts = Counter(case["disposition"] for case in ledger)

    if len(ledger) != 57:
        errors.append(f"spatial ledger has {len(ledger)} cases, expected 57")
    if counts != Counter(EXPECTED_COUNTS):
        errors.append(f"spatial dispositions are {dict(counts)}, expected {EXPECTED_COUNTS}")
    executable = sum(counts[name] for name in EXECUTABLE)
    if executable != 42:
        errors.append(f"spatial ledger has {executable} executable cases, expected 42")
    if len(names) != len(set(names)):
        errors.append("spatial ledger contains duplicate case names")
    if set(names) != set(fixture_names):
        missing = sorted(set(names) - set(fixture_names))
        extra = sorted(set(fixture_names) - set(names))
        errors.append(f"spatial fixture drift: missing={missing}, extra={extra}")


def check_claim_text(errors: list[str]) -> None:
    required = {
        "README.md": "42 curated spatial cases",
        "ROADMAP.md": "42 native/rewrite/macro cases",
        "docs/ROADMAP_STATUS.md": "42 original PostGIS expressions",
        "docs/COMPATIBILITY.md": "42 curated spatial cases",
        "docs/PROJECT_DIRECTION.md": "Forty-two native, rewrite, or macro spatial cases",
    }
    for relative, phrase in required.items():
        text = (ROOT / relative).read_text(encoding="utf-8")
        if phrase not in text:
            errors.append(f"{relative}: missing maintained claim {phrase!r}")


def main() -> int:
    errors: list[str] = []
    try:
        check_markdown(errors)
        check_spatial_ledger(errors)
        check_claim_text(errors)
    except (OSError, subprocess.CalledProcessError, ValueError, json.JSONDecodeError) as error:
        errors.append(str(error))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("project_contract_check_ok markdown=tracked spatial=57 executable=42")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
