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
POSTGRESQL_PROFILE = ROOT / "tests/fixtures/postgresql18_compatibility_profile.json"
POSTGRESQL_REFERENCE = ROOT / "tests/fixtures/postgresql18_column_core_reference.json"


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


def check_postgresql_profile(errors: list[str]) -> None:
    profile = json.loads(POSTGRESQL_PROFILE.read_text(encoding="utf-8"))
    reference = json.loads(POSTGRESQL_REFERENCE.read_text(encoding="utf-8"))

    if profile.get("schema_version") != 1:
        errors.append("PostgreSQL compatibility profile schema_version must be 1")
    if profile.get("profile_id") != "pg18-column-core-v1":
        errors.append("PostgreSQL compatibility profile_id must be pg18-column-core-v1")
    if profile.get("target", {}).get("postgresql_major") != 18:
        errors.append("PostgreSQL compatibility profile must target major version 18")
    if reference.get("profile_id") != profile.get("profile_id"):
        errors.append("PostgreSQL reference output does not name the active profile")
    digest = reference.get("oracle", {}).get("image_digest", "")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        errors.append("PostgreSQL reference image digest is missing or malformed")

    relations = profile.get("catalog_relations", [])
    relation_names = [relation.get("name") for relation in relations]
    required_relations = {
        "pg_catalog.pg_namespace",
        "pg_catalog.pg_type",
        "pg_catalog.pg_range",
        "pg_catalog.pg_class",
        "pg_catalog.pg_attribute",
        "pg_catalog.pg_database",
        "information_schema.tables",
        "information_schema.columns",
    }
    if len(relation_names) != len(set(relation_names)):
        errors.append("PostgreSQL compatibility profile contains duplicate relations")
    missing_relations = sorted(required_relations - set(relation_names))
    if missing_relations:
        errors.append(f"PostgreSQL compatibility profile missing relations: {missing_relations}")
    for relation in relations:
        columns = relation.get("required_columns", [])
        names = [column.get("name") for column in columns]
        if not relation.get("stage") or not relation.get("trace_status") or not columns:
            errors.append(f"PostgreSQL relation is incomplete: {relation.get('name')!r}")
        if len(names) != len(set(names)):
            errors.append(f"PostgreSQL relation has duplicate columns: {relation.get('name')!r}")
        for column in columns:
            if not isinstance(column.get("type_oid"), int) or column["type_oid"] <= 0:
                errors.append(
                    f"PostgreSQL relation column has invalid type OID: "
                    f"{relation.get('name')}.{column.get('name')}"
                )

    queries = profile.get("query_families", [])
    query_by_id = {query.get("id"): query for query in queries}
    if len(query_by_id) != len(queries) or None in query_by_id:
        errors.append("PostgreSQL compatibility profile contains duplicate/unnamed queries")
    for query in queries:
        if not query.get("sql") or not query.get("consumers") or not query.get("expected_columns"):
            errors.append(f"PostgreSQL query family is incomplete: {query.get('id')!r}")
        column_names = [column.get("name") for column in query.get("expected_columns", [])]
        if len(column_names) != len(set(column_names)):
            errors.append(f"PostgreSQL query has duplicate result columns: {query.get('id')!r}")

    for description in reference.get("query_descriptions", []):
        query_id = description.get("query_id")
        query = query_by_id.get(query_id)
        if query is None:
            errors.append(f"PostgreSQL reference has unknown query_id: {query_id!r}")
            continue
        expected = [
            (column.get("name"), column.get("type_oid"))
            for column in query.get("expected_columns", [])
        ]
        actual = [
            (column.get("name"), column.get("type_oid"))
            for column in description.get("columns", [])
        ]
        if actual != expected:
            errors.append(f"PostgreSQL reference result drift for query: {query_id}")

    pending = profile.get("pending_trace_families", [])
    pending_ids = [item.get("id") for item in pending]
    if len(pending_ids) != len(set(pending_ids)) or not all(pending_ids):
        errors.append("PostgreSQL compatibility profile has duplicate/unnamed pending traces")


def main() -> int:
    errors: list[str] = []
    try:
        check_markdown(errors)
        check_spatial_ledger(errors)
        check_claim_text(errors)
        check_postgresql_profile(errors)
    except (OSError, subprocess.CalledProcessError, ValueError, json.JSONDecodeError) as error:
        errors.append(str(error))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(
        "project_contract_check_ok markdown=tracked spatial=57 executable=42 "
        "postgresql_profile=pg18-column-core-v1"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
