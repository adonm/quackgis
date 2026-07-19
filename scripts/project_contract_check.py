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
    "quackgis_macro": 8,
    "rust_edge": 8,
    "extension_candidate": 5,
}
POSTGRESQL_PROFILE = ROOT / "tests/fixtures/postgresql18_compatibility_profile.json"
POSTGRESQL_REFERENCE = ROOT / "tests/fixtures/postgresql18_column_core_reference.json"
OGR_TRACE = ROOT / "tests/fixtures/ogr_3_11_5_postgresql18_trace.json"
PSQL_TRACE = ROOT / "tests/fixtures/psql_18_3_postgresql18_describe_trace.json"
QGIS_TRACE = ROOT / "tests/fixtures/qgis_3_44_postgresql18_trace.json"


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
    if executable != 44:
        errors.append(f"spatial ledger has {executable} executable cases, expected 44")
    if len(names) != len(set(names)):
        errors.append("spatial ledger contains duplicate case names")
    if set(names) != set(fixture_names):
        missing = sorted(set(names) - set(fixture_names))
        extra = sorted(set(fixture_names) - set(names))
        errors.append(f"spatial fixture drift: missing={missing}, extra={extra}")


def check_claim_text(errors: list[str]) -> None:
    required = {
        "README.md": "44 curated spatial cases",
        "ROADMAP.md": "44 native/rewrite/macro cases",
        "docs/ROADMAP_STATUS.md": "44 original PostGIS expressions",
        "docs/COMPATIBILITY.md": "44 curated spatial cases",
        "docs/PROJECT_DIRECTION.md": "Forty-four native, rewrite, or macro spatial cases",
    }
    for relative, phrase in required.items():
        text = (ROOT / relative).read_text(encoding="utf-8")
        if phrase not in text:
            errors.append(f"{relative}: missing maintained claim {phrase!r}")


def check_postgresql_profile(errors: list[str]) -> None:
    profile = json.loads(POSTGRESQL_PROFILE.read_text(encoding="utf-8"))
    reference = json.loads(POSTGRESQL_REFERENCE.read_text(encoding="utf-8"))
    ogr_trace = json.loads(OGR_TRACE.read_text(encoding="utf-8"))
    psql_trace = json.loads(PSQL_TRACE.read_text(encoding="utf-8"))
    qgis_trace = json.loads(QGIS_TRACE.read_text(encoding="utf-8"))

    if profile.get("schema_version") != 1:
        errors.append("PostgreSQL compatibility profile schema_version must be 1")
    if profile.get("profile_id") != "pg18-column-core-v1":
        errors.append("PostgreSQL compatibility profile_id must be pg18-column-core-v1")
    if profile.get("target", {}).get("postgresql_major") != 18:
        errors.append("PostgreSQL compatibility profile must target major version 18")
    if profile.get("target", {}).get("postgresql_version") != "18.4":
        errors.append("PostgreSQL compatibility profile must target version 18.4")
    identity = profile.get("identity_policy", {})
    if [
        identity.get("geometry_oid"),
        identity.get("geography_oid"),
        identity.get("geometry_array_oid"),
        identity.get("geography_array_oid"),
        identity.get("postgis_lib_version_proc_oid"),
        identity.get("postgis_version_proc_oid"),
        identity.get("postgis_geos_version_proc_oid"),
        identity.get("postgis_proj_version_proc_oid"),
    ] != [90_001, 90_002, 90_003, 90_004, 90_005, 90_006, 90_007, 90_008]:
        errors.append("PostGIS type/routine compatibility OIDs drifted")
    if reference.get("profile_id") != profile.get("profile_id"):
        errors.append("PostgreSQL reference output does not name the active profile")
    digest = reference.get("oracle", {}).get("image_digest", "")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        errors.append("PostgreSQL reference image digest is missing or malformed")
    builtin_types = [
        (item.get("oid"), item.get("name")) for item in reference.get("builtin_types", [])
    ]
    builtin_rows = reference.get("builtin_type_rows", [])
    if len(builtin_types) != 24 or len(builtin_rows) != 24:
        errors.append("PostgreSQL built-in type oracle must contain 24 profile/QGIS rows")
    if any(not isinstance(row, list) or len(row) != 18 for row in builtin_rows):
        errors.append("PostgreSQL built-in type oracle rows must contain all 18 fields")
    elif [(row[0], row[1]) for row in builtin_rows] != builtin_types:
        errors.append("PostgreSQL built-in type names drifted from full oracle rows")

    relations = profile.get("catalog_relations", [])
    relation_names = [relation.get("name") for relation in relations]
    required_relations = {
        "pg_catalog.pg_namespace",
        "pg_catalog.pg_proc",
        "pg_catalog.pg_type",
        "pg_catalog.pg_range",
        "pg_catalog.pg_collation",
        "pg_catalog.pg_class",
        "pg_catalog.pg_attribute",
        "pg_catalog.pg_attrdef",
        "pg_catalog.pg_description",
        "pg_catalog.pg_constraint",
        "pg_catalog.pg_index",
        "geometry_columns",
        "spatial_ref_sys",
        "pg_catalog.pg_database",
        "pg_catalog.pg_roles",
        "information_schema.tables",
        "information_schema.columns",
        "information_schema.schemata",
        "information_schema.table_privileges",
        "information_schema.role_table_grants",
        "information_schema.column_privileges",
        "information_schema.role_column_grants",
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

    captured = profile.get("captured_trace_families", [])
    captured_ids = [item.get("id") for item in captured]
    if "ogr_copied_spatial_table" not in captured_ids:
        errors.append("PostgreSQL compatibility profile does not register the OGR trace")
    if "psql_describe_copied_table" not in captured_ids:
        errors.append("PostgreSQL compatibility profile does not register the psql trace")
    if "qgis_headless_copied_layer" not in captured_ids:
        errors.append("PostgreSQL compatibility profile does not register the QGIS trace")
    if set(captured_ids) & set(pending_ids):
        errors.append("PostgreSQL trace cannot be both captured and pending")

    if ogr_trace.get("trace_id") != "ogr-3.11.5-postgresql18-copied-point-v1":
        errors.append("OGR trace_id is missing or unsupported")
    if ogr_trace.get("client", {}).get("gdal_ogr_version") != "3.11.5":
        errors.append("OGR trace must use GDAL/OGR 3.11.5")
    if ogr_trace.get("oracle", {}).get("postgresql_version") != "18.4":
        errors.append("OGR trace must use the PostgreSQL 18.4 oracle")
    for label, value in [
        ("OGR client image", ogr_trace.get("client", {}).get("image_digest", "")),
        ("PostGIS image", ogr_trace.get("oracle", {}).get("postgis_image_digest", "")),
    ]:
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
            errors.append(f"{label} digest is missing or malformed")
    serialized_trace = json.dumps(ogr_trace).lower()
    for secret in ["password=", "reference-only"]:
        if secret in serialized_trace:
            errors.append(f"OGR trace contains credential material: {secret}")
    ogr_queries = ogr_trace.get("queries", [])
    ogr_query_ids = [query.get("id") for query in ogr_queries]
    if len(ogr_query_ids) != len(set(ogr_query_ids)) or len(ogr_queries) < 20:
        errors.append("OGR trace query corpus is duplicate, unnamed, or incomplete")
    required_ogr_queries = {
        "find_postgis_namespace",
        "discover_spatial_type_oids",
        "empty_geometry_srid",
        "relation_oid",
        "primary_key_columns",
        "column_structure",
        "geometry_column_metadata",
        "spatial_extent",
        "spatial_reference",
    }
    missing_ogr_queries = sorted(required_ogr_queries - set(ogr_query_ids))
    if missing_ogr_queries:
        errors.append(f"OGR trace missing required query families: {missing_ogr_queries}")
    ogr_query_by_id = {query.get("id"): query for query in ogr_queries}
    profile_namespace_query = query_by_id.get("find_postgis_namespace", {}).get("sql")
    trace_namespace_query = ogr_query_by_id.get("find_postgis_namespace", {}).get("sql")
    if profile_namespace_query != trace_namespace_query:
        errors.append("PostgreSQL profile find_postgis_namespace SQL drifted from OGR trace")
    if query_by_id.get("column_structure", {}).get("sql") != ogr_query_by_id.get(
        "column_structure", {}
    ).get("sql"):
        errors.append("PostgreSQL profile column_structure SQL drifted from OGR trace")
    if query_by_id.get("primary_key_columns", {}).get("sql") != ogr_query_by_id.get(
        "primary_key_columns", {}
    ).get("sql"):
        errors.append("PostgreSQL profile primary_key_columns SQL drifted from OGR trace")

    if psql_trace.get("trace_id") != "psql-18.3-postgresql18-describe-spatial-table-v1":
        errors.append("psql trace_id is missing or unsupported")
    if psql_trace.get("client", {}).get("psql_version") != "18.3":
        errors.append("psql trace must use psql 18.3")
    if psql_trace.get("oracle", {}).get("postgresql_version") != "18.4":
        errors.append("psql trace must use the PostgreSQL 18.4 oracle")
    for label, value in [
        ("psql client image", psql_trace.get("client", {}).get("image_digest", "")),
        ("psql PostGIS image", psql_trace.get("oracle", {}).get("postgis_image_digest", "")),
    ]:
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
            errors.append(f"{label} digest is missing or malformed")
    serialized_psql_trace = json.dumps(psql_trace).lower()
    for secret in ["password=", "reference-only"]:
        if secret in serialized_psql_trace:
            errors.append(f"psql trace contains credential material: {secret}")
    psql_queries = psql_trace.get("queries", [])
    psql_query_ids = [query.get("id") for query in psql_queries]
    if len(psql_query_ids) != len(set(psql_query_ids)) or len(psql_queries) != 12:
        errors.append("psql trace query corpus is duplicate, unnamed, or incomplete")
    required_psql_queries = {
        "resolve_relation",
        "relation_properties",
        "column_properties",
        "indexes",
        "not_null_constraints",
        "row_security_policies",
        "publications",
        "partition_children",
    }
    missing_psql_queries = sorted(required_psql_queries - set(psql_query_ids))
    if missing_psql_queries:
        errors.append(f"psql trace missing required query families: {missing_psql_queries}")
    psql_query_by_id = {query.get("id"): query for query in psql_queries}
    if query_by_id.get("resolve_relation", {}).get("sql") != psql_query_by_id.get(
        "resolve_relation", {}
    ).get("sql"):
        errors.append("PostgreSQL profile resolve_relation SQL drifted from psql trace")

    if qgis_trace.get("trace_id") != "qgis-3.44.11-postgresql18-read-point-v1":
        errors.append("QGIS trace_id is missing or unsupported")
    if qgis_trace.get("client", {}).get("qgis_version") != "3.44.11-Solothurn":
        errors.append("QGIS trace must use QGIS 3.44.11")
    if qgis_trace.get("oracle", {}).get("postgresql_version") != "18.4":
        errors.append("QGIS trace must use the PostgreSQL 18.4 oracle")
    for label, value in [
        ("QGIS client image", qgis_trace.get("client", {}).get("image_digest", "")),
        ("QGIS PostGIS image", qgis_trace.get("oracle", {}).get("postgis_image_digest", "")),
    ]:
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
            errors.append(f"{label} digest is missing or malformed")
    serialized_qgis_trace = json.dumps(qgis_trace).lower()
    for secret in ["password=", "reference-only"]:
        if secret in serialized_qgis_trace:
            errors.append(f"QGIS trace contains credential material: {secret}")
    qgis_queries = qgis_trace.get("queries", [])
    qgis_query_ids = [query.get("id") for query in qgis_queries]
    if (
        len(qgis_query_ids) != len(set(qgis_query_ids))
        or len(qgis_queries) != 26
        or qgis_trace.get("statement_count") != 32
        or qgis_trace.get("unique_query_count") != 26
    ):
        errors.append("QGIS trace query corpus/counts are duplicate, unnamed, or incomplete")
    required_qgis_queries = {
        "layer_privileges",
        "owner_membership",
        "geometry_metadata",
        "attribute_structure",
        "field_type_resolution",
        "identity_index",
        "spatial_reference",
        "spatial_extent",
        "binary_read_cursor",
        "binary_read_fetch",
        "binary_read_close",
    }
    missing_qgis_queries = sorted(required_qgis_queries - set(qgis_query_ids))
    if missing_qgis_queries:
        errors.append(f"QGIS trace missing required query families: {missing_qgis_queries}")
    qgis_query_by_id = {query.get("id"): query for query in qgis_queries}
    if query_by_id.get("attribute_structure", {}).get("sql") != qgis_query_by_id.get(
        "attribute_structure", {}
    ).get("sql"):
        errors.append("PostgreSQL profile attribute_structure SQL drifted from QGIS trace")


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
        "project_contract_check_ok markdown=tracked spatial=57 executable=44 "
        "postgresql_profile=pg18-column-core-v1"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
