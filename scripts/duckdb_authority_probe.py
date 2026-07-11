#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run a DuckDB-authored DuckLake vertical-slice probe.

DuckDB creates the DuckLake catalog, writes WKB rows, mutates them, reopens the
catalog, and emits a small evidence manifest. It is an independent check of the
official storage-authority contract.
"""

from __future__ import annotations

import argparse
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


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def run_sql(binary: str, sql: str) -> SqlResult:
    result = subprocess.run(
        [binary, "-csv", ":memory:", "-c", sql],
        text=True,
        capture_output=True,
        check=False,
    )
    stdout = redact(result.stdout)
    stderr = redact(result.stderr)
    return SqlResult(
        status="pass" if result.returncode == 0 else "fail",
        stdout=stdout,
        stderr=stderr,
        returncode=result.returncode,
    )


def create_sql(catalog: Path, data_path: Path) -> str:
    catalog_literal = sql_literal(f"ducklake:{catalog}")
    data_literal = sql_literal(str(data_path))
    return f"""
LOAD spatial;
LOAD ducklake;
SET ducklake_default_data_inlining_row_limit = 0;
ATTACH {catalog_literal} AS qg (
  DATA_PATH {data_literal},
  DATA_INLINING_ROW_LIMIT 0
);
USE qg;
CREATE SCHEMA public;
CREATE TABLE public.points (
  id INTEGER,
  name VARCHAR,
  geom_wkb BLOB,
  _qg_minx DOUBLE,
  _qg_miny DOUBLE,
  _qg_maxx DOUBLE,
  _qg_maxy DOUBLE
);
INSERT INTO public.points VALUES
  (1, 'origin', ST_AsWKB(ST_GeomFromText('POINT (0 0)')), 0, 0, 0, 0),
  (2, 'one', ST_AsWKB(ST_GeomFromText('POINT (1 1)')), 1, 1, 1, 1),
  (3, 'two', ST_AsWKB(ST_GeomFromText('POINT (2 2)')), 2, 2, 2, 2);
SELECT 'after_insert' AS check_name, COUNT(*) AS rows FROM public.points;
UPDATE public.points SET name = 'uno' WHERE id = 2;
DELETE FROM public.points WHERE id = 1;
SELECT
  'after_mutation' AS check_name,
  COUNT(*) AS rows,
  STRING_AGG(name, ',' ORDER BY id) AS names
FROM public.points;
FROM ducklake_table_info('qg');
"""


def reopen_sql(catalog: Path) -> str:
    catalog_literal = sql_literal(f"ducklake:{catalog}")
    return f"""
LOAD spatial;
LOAD ducklake;
ATTACH {catalog_literal} AS qg;
USE qg;
SELECT
  'reopen' AS check_name,
  COUNT(*) AS rows,
  STRING_AGG(name, ',' ORDER BY id) AS names,
  SUM(
    CASE
      WHEN ST_Intersects(
        ST_GeomFromWKB(geom_wkb),
        ST_GeomFromText('POLYGON ((0.5 0.5, 3 0.5, 3 3, 0.5 3, 0.5 0.5))')
      ) THEN 1 ELSE 0
    END
  ) AS hits
FROM public.points;
FROM ducklake_snapshots('qg');
"""


def validate(create: SqlResult, reopen: SqlResult) -> dict[str, str]:
    checks = {
        "create_insert": "fail",
        "mutation": "fail",
        "metadata": "fail",
        "reopen": "fail",
        "spatial_wkb": "fail",
        "snapshot_metadata": "fail",
    }
    if create.returncode == 0 and "after_insert,3" in create.stdout:
        checks["create_insert"] = "pass"
    if create.returncode == 0 and 'after_mutation,2,"uno,two"' in create.stdout:
        checks["mutation"] = "pass"
    if create.returncode == 0 and "table_name" in create.stdout and "points" in create.stdout:
        checks["metadata"] = "pass"
    if reopen.returncode == 0 and 'reopen,2,"uno,two",2' in reopen.stdout:
        checks["reopen"] = "pass"
        checks["spatial_wkb"] = "pass"
    if reopen.returncode == 0 and "snapshot_id" in reopen.stdout and "tables_deleted_from" in reopen.stdout:
        checks["snapshot_metadata"] = "pass"
    return checks


def git_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], text=True, capture_output=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def data_stats(data_path: Path) -> dict[str, int]:
    files = [path for path in data_path.glob("**/*") if path.is_file()]
    parquet = [path for path in files if path.suffix == ".parquet"]
    return {
        "file_count": len(files),
        "parquet_count": len(parquet),
        "object_bytes": sum(path.stat().st_size for path in files),
    }


def manifest(
    *,
    source_sha: str,
    duckdb_version: str,
    catalog: Path,
    data_path: Path,
    checks: dict[str, str],
    stats: dict[str, int],
) -> dict[str, Any]:
    return {
        "source_sha": source_sha,
        "claim": "duckdb_storage_authority_vertical_slice",
        "storage_profile": "duckdb-local-ducklake",
        "duckdb_version": duckdb_version,
        "catalog": str(catalog),
        "data_path": str(data_path),
        "checks": checks,
        "final_row_count": 2,
        "final_names": ["uno", "two"],
        **stats,
    }


def render_report(
    *,
    summary: dict[str, Any],
    create: SqlResult | None = None,
    reopen: SqlResult | None = None,
    message: str | None = None,
) -> str:
    passed = sum(1 for status in summary.get("checks", {}).values() if status == "pass")
    total = len(summary.get("checks", {}))
    body = [
        "# DuckDB storage-authority probe",
        "",
        f"Status: `{'pass' if passed == total and total else 'fail'}`",
        f"Source SHA: `{summary.get('source_sha', 'unknown')}`",
        f"DuckDB version: `{summary.get('duckdb_version', 'unknown')}`",
        f"Catalog: `{summary.get('catalog', '')}`",
        f"Data path: `{summary.get('data_path', '')}`",
        f"Checks: {passed}/{total} passed",
        f"Objects: files={summary.get('file_count', 0)} parquet={summary.get('parquet_count', 0)} bytes={summary.get('object_bytes', 0)}",
        "",
        "| Check | Status |",
        "|---|---|",
    ]
    for check, status in summary.get("checks", {}).items():
        body.append(f"| `{check}` | {status} |")
    if message:
        body.extend(["", message])
    for label, result in (("Create/mutate", create), ("Reopen", reopen)):
        if result is None:
            continue
        body.extend(
            [
                "",
                f"## {label} SQL output",
                "",
                f"Status: `{result.status}` returncode={result.returncode}",
                "",
                "```text",
                result.stdout.strip(),
                "```",
            ]
        )
        if result.stderr.strip():
            body.extend(["", "stderr:", "", "```text", result.stderr.strip(), "```"])
    body.append("")
    return "\n".join(body)


def prepare_workdir(workdir: Path) -> tuple[Path, Path]:
    if workdir.exists():
        shutil.rmtree(workdir)
    data_path = workdir / "data"
    data_path.mkdir(parents=True, exist_ok=True)
    return workdir / "catalog.ducklake", data_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb-bin", default="duckdb")
    parser.add_argument("--workdir", type=Path, default=Path(".tmp/duckdb-authority"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args(argv)

    args.out.unlink(missing_ok=True)
    args.manifest.unlink(missing_ok=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)

    binary = resolve_binary(args.duckdb_bin)
    if not binary:
        summary = {
            "source_sha": git_sha(),
            "duckdb_version": "missing",
            "catalog": str(args.workdir / "catalog.ducklake"),
            "data_path": str(args.workdir / "data"),
            "checks": {"duckdb_available": "fail"},
        }
        args.manifest.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        args.out.write_text(
            render_report(summary=summary, message="Install DuckDB CLI or set `DUCKDB_BIN`."),
            encoding="utf-8",
        )
        print(f"duckdb_authority_probe_missing out={args.out}", file=sys.stderr)
        return 2

    try:
        version = run_version(binary)
        catalog, data_path = prepare_workdir(args.workdir)
        create = run_sql(binary, create_sql(catalog.resolve(), data_path.resolve()))
        reopen = run_sql(binary, reopen_sql(catalog.resolve()))
        checks = validate(create, reopen)
        summary = manifest(
            source_sha=git_sha(),
            duckdb_version=version,
            catalog=catalog,
            data_path=data_path,
            checks=checks,
            stats=data_stats(data_path),
        )
        args.manifest.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        args.out.write_text(render_report(summary=summary, create=create, reopen=reopen), encoding="utf-8")
    except (OSError, RuntimeError) as error:
        summary = {
            "source_sha": git_sha(),
            "duckdb_version": "error",
            "catalog": str(args.workdir / "catalog.ducklake"),
            "data_path": str(args.workdir / "data"),
            "checks": {"probe_error": "fail"},
        }
        args.manifest.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        args.out.write_text(
            render_report(summary=summary, message=redact(str(error))), encoding="utf-8"
        )
        print(f"duckdb authority probe failed: {error}", file=sys.stderr)
        return 1

    if all(status == "pass" for status in checks.values()):
        print(f"duckdb_authority_probe_ok out={args.out} manifest={args.manifest}")
        return 0
    print(f"duckdb_authority_probe_failed out={args.out} manifest={args.manifest}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
