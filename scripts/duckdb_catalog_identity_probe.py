#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Prove durable DuckLake identity needed by the PostgreSQL catalog projection."""

from __future__ import annotations

import argparse
import csv
import io
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, NamedTuple

from duckdb_engine_probe import redact, resolve_binary, run_version


class SqlResult(NamedTuple):
    status: str
    stdout: str
    stderr: str
    returncode: int


Identity = tuple[int, int, str]


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def run_sql(binary: str, sql: str) -> SqlResult:
    result = subprocess.run(
        [binary, "-csv", ":memory:", "-c", sql],
        text=True,
        capture_output=True,
        check=False,
    )
    return SqlResult(
        status="pass" if result.returncode == 0 else "fail",
        stdout=redact(result.stdout),
        stderr=redact(result.stderr),
        returncode=result.returncode,
    )


def tagged_rows(result: SqlResult, tag: str, width: int) -> list[list[str]]:
    rows = []
    for row in csv.reader(io.StringIO(result.stdout)):
        if len(row) == width and row[0] == tag:
            rows.append(row)
    return rows


def one_identity(result: SqlResult, tag: str) -> Identity | None:
    rows = tagged_rows(result, tag, 4)
    if len(rows) != 1:
        return None
    try:
        schema_id = int(rows[0][1])
        table_id = int(rows[0][2])
    except ValueError:
        return None
    table_uuid = rows[0][3]
    if schema_id < 0 or table_id < 0 or not table_uuid:
        return None
    return (schema_id, table_id, table_uuid)


def create_and_rename_sql(catalog: Path, data_path: Path) -> str:
    return f"""
LOAD ducklake;
SET ducklake_default_data_inlining_row_limit = 0;
ATTACH {sql_literal(f'ducklake:{catalog}')} AS qg (
  DATA_PATH {sql_literal(str(data_path))},
  DATA_INLINING_ROW_LIMIT 0
);
CREATE SCHEMA qg.id_schema;
CREATE TABLE qg.id_schema.before_name(id INTEGER, old_column VARCHAR);
INSERT INTO qg.id_schema.before_name VALUES (1, 'before');
SELECT
  'identity_before' AS check_name,
  schema_id,
  table_id,
  CAST(table_uuid AS VARCHAR) AS table_uuid
FROM ducklake_table_info('qg')
WHERE table_name = 'before_name';
ALTER TABLE qg.id_schema.before_name RENAME TO after_name;
ALTER TABLE qg.id_schema.after_name RENAME COLUMN old_column TO new_column;
INSERT INTO qg.id_schema.after_name VALUES (2, 'after');
SELECT
  'identity_after' AS check_name,
  schema_id,
  table_id,
  CAST(table_uuid AS VARCHAR) AS table_uuid
FROM ducklake_table_info('qg')
WHERE table_name = 'after_name';
SELECT 'identity_file' AS check_name, data_file
FROM ducklake_list_files('qg', 'after_name', schema => 'id_schema')
ORDER BY data_file;
"""


def field_identity_sql(data_file: str) -> str:
    return f"""
SELECT 'field_identity' AS check_name, name, field_id
FROM parquet_schema({sql_literal(data_file)})
WHERE name IN ('old_column', 'new_column')
ORDER BY name;
"""


def reopen_sql(catalog: Path) -> str:
    return f"""
LOAD ducklake;
ATTACH {sql_literal(f'ducklake:{catalog}')} AS qg (READ_ONLY);
SELECT
  'identity_reopen' AS check_name,
  schema_id,
  table_id,
  CAST(table_uuid AS VARCHAR) AS table_uuid
FROM ducklake_table_info('qg')
WHERE table_name = 'after_name';
SELECT
  'column_reopen' AS check_name,
  column_name,
  ordinal_position
FROM information_schema.columns
WHERE table_catalog = 'qg'
  AND table_schema = 'id_schema'
  AND table_name = 'after_name'
ORDER BY ordinal_position;
SELECT
  'rows_reopen' AS check_name,
  COUNT(*) AS rows,
  STRING_AGG(new_column, ',' ORDER BY id) AS values
FROM qg.id_schema.after_name;
"""


def recreate_sql(catalog: Path) -> str:
    return f"""
LOAD ducklake;
ATTACH {sql_literal(f'ducklake:{catalog}')} AS qg;
DROP TABLE qg.id_schema.after_name;
CREATE TABLE qg.id_schema.after_name(id INTEGER, new_column VARCHAR);
SELECT
  'identity_recreated' AS check_name,
  schema_id,
  table_id,
  CAST(table_uuid AS VARCHAR) AS table_uuid
FROM ducklake_table_info('qg')
WHERE table_name = 'after_name';
"""


def validate(
    create: SqlResult,
    fields: list[SqlResult],
    reopen: SqlResult,
    recreate: SqlResult,
) -> tuple[dict[str, str], dict[str, Any]]:
    before = one_identity(create, "identity_before")
    after = one_identity(create, "identity_after")
    reopened = one_identity(reopen, "identity_reopen")
    recreated = one_identity(recreate, "identity_recreated")

    field_rows = [
        row
        for result in fields
        for row in tagged_rows(result, "field_identity", 3)
    ]
    field_ids: dict[str, set[int]] = {}
    for _, name, raw_id in field_rows:
        try:
            field_id = int(raw_id)
        except ValueError:
            continue
        field_ids.setdefault(name, set()).add(field_id)

    old_ids = field_ids.get("old_column", set())
    new_ids = field_ids.get("new_column", set())
    columns = tagged_rows(reopen, "column_reopen", 3)
    row_oracle = tagged_rows(reopen, "rows_reopen", 3)

    checks = {
        "table_rename_identity": "pass"
        if before is not None and before == after
        else "fail",
        "independent_reopen_identity": "pass"
        if before is not None and before == reopened
        else "fail",
        "column_rename_field_identity": "pass"
        if len(old_ids) == 1 and old_ids == new_ids and next(iter(old_ids)) > 0
        else "fail",
        "current_names_and_rows": "pass"
        if columns == [["column_reopen", "id", "1"], ["column_reopen", "new_column", "2"]]
        and row_oracle == [["rows_reopen", "2", "before,after"]]
        else "fail",
        "drop_recreate_new_identity": "pass"
        if before is not None
        and recreated is not None
        and before[0] == recreated[0]
        and (before[1], before[2]) != (recreated[1], recreated[2])
        else "fail",
    }
    evidence = {
        "before": before,
        "after_rename": after,
        "after_reopen": reopened,
        "after_drop_recreate": recreated,
        "column_field_ids": {
            name: sorted(values) for name, values in sorted(field_ids.items())
        },
    }
    return checks, evidence


def prepare_workdir(workdir: Path) -> tuple[Path, Path]:
    if workdir.exists():
        shutil.rmtree(workdir)
    data_path = workdir / "data"
    data_path.mkdir(parents=True, exist_ok=True)
    return workdir / "catalog.ducklake", data_path


def git_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], text=True, capture_output=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def render_report(summary: dict[str, Any], results: list[tuple[str, SqlResult]]) -> str:
    checks = summary.get("checks", {})
    passed = sum(status == "pass" for status in checks.values())
    body = [
        "# DuckLake catalog identity probe",
        "",
        f"Status: `{'pass' if passed == len(checks) and checks else 'fail'}`",
        f"Source SHA: `{summary.get('source_sha', 'unknown')}`",
        f"DuckDB version: `{summary.get('duckdb_version', 'unknown')}`",
        f"Checks: {passed}/{len(checks)} passed",
        "",
        "Decision: durable DuckLake table UUID/ID and column field IDs are",
        "compatibility-registry keys, not PostgreSQL OIDs. A small transactional",
        "registry remains required for nonzero uint32 namespace/relation OIDs and",
        "durable PostgreSQL attribute numbers.",
        "",
        "Public API boundary: `ducklake_table_info` exposes durable table identity,",
        "but no complete supported function exposes qualified schema UUIDs and all",
        "column IDs, including empty/new columns. C2 catalog extraction therefore",
        "remains blocked on a public DuckLake API or a separately approved pinned",
        "specification adapter.",
        "",
        "| Check | Status |",
        "|---|---|",
    ]
    body.extend(f"| `{name}` | {status} |" for name, status in checks.items())
    body.extend(["", "## Identity evidence", "", "```json"])
    body.append(json.dumps(summary.get("identity_evidence", {}), indent=2))
    body.extend(["```", ""])
    for label, result in results:
        body.extend(
            [
                f"## {label}",
                "",
                f"Status: `{result.status}` returncode={result.returncode}",
                "",
                "```text",
                result.stdout.strip(),
                "```",
                "",
            ]
        )
        if result.stderr.strip():
            body.extend(["stderr:", "", "```text", result.stderr.strip(), "```", ""])
    return "\n".join(body)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb-bin", default="duckdb")
    parser.add_argument("--workdir", type=Path, default=Path(".tmp/duckdb-catalog-identity"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args(argv)

    args.out.unlink(missing_ok=True)
    args.manifest.unlink(missing_ok=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)

    binary = resolve_binary(args.duckdb_bin)
    if not binary:
        print("DuckDB CLI is missing", file=sys.stderr)
        return 2

    try:
        catalog, data_path = prepare_workdir(args.workdir)
        catalog = catalog.resolve()
        data_path = data_path.resolve()
        create = run_sql(binary, create_and_rename_sql(catalog, data_path))
        files = [row[1] for row in tagged_rows(create, "identity_file", 2)]
        field_results = [run_sql(binary, field_identity_sql(path)) for path in files]
        reopen = run_sql(binary, reopen_sql(catalog))
        recreate = run_sql(binary, recreate_sql(catalog))
        checks, identity_evidence = validate(create, field_results, reopen, recreate)
        summary = {
            "source_sha": git_sha(),
            "claim": "ducklake_durable_catalog_identity_feasibility",
            "duckdb_version": run_version(binary),
            "storage_profile": "duckdb-local-ducklake",
            "checks": checks,
            "identity_evidence": identity_evidence,
            "decision": {
                "ducklake_identity": "durable registry key",
                "postgresql_oid": "transactional compatibility registry required",
                "column_api": "public DuckLake catalog identity API gap",
                "schema_rename": "unsupported by current DuckLake surface",
            },
        }
        results = [("Create and rename", create)]
        results.extend(
            (f"Parquet field identity {index + 1}", result)
            for index, result in enumerate(field_results)
        )
        results.extend([("Independent reopen", reopen), ("Drop and recreate", recreate)])
        args.manifest.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        args.out.write_text(render_report(summary, results), encoding="utf-8")
    except (OSError, RuntimeError, ValueError) as error:
        print(f"DuckLake catalog identity probe failed: {redact(str(error))}", file=sys.stderr)
        return 1

    if checks and all(status == "pass" for status in checks.values()):
        print(f"duckdb_catalog_identity_probe_ok out={args.out} manifest={args.manifest}")
        return 0
    print(f"duckdb_catalog_identity_probe_failed out={args.out}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
