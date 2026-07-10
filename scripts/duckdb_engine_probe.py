#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run the first DuckDB engine feasibility probe.

The probe is intentionally out-of-process: it exercises DuckDB's `spatial` and
`ducklake` extensions through the CLI without adding DuckDB to the QuackGIS server
runtime. It can also run caller-provided attach SQL against a copied DuckLake
catalog/prefix when a reference-readability experiment is available.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path
import re
from typing import NamedTuple


ENGINE_SQL = """
INSTALL spatial;
LOAD spatial;
INSTALL ducklake;
LOAD ducklake;

CREATE TABLE qg_points AS
SELECT *
FROM (
  VALUES
    (1, 'origin', ST_AsWKB(ST_GeomFromText('POINT (0 0)'))),
    (2, 'one', ST_AsWKB(ST_GeomFromText('POINT (1 1)')))
) AS t(id, name, geom_wkb);

SELECT 'ducklake_extension_loaded' AS check_name, 1 AS ok;
SELECT
  'engine_spatial_wkb' AS check_name,
  COUNT(*) AS rows,
  SUM(
    CASE
      WHEN ST_Intersects(
        ST_GeomFromWKB(geom_wkb),
        ST_GeomFromText('POLYGON ((-1 -1, 2 -1, 2 2, -1 2, -1 -1))')
      ) THEN 1 ELSE 0
    END
  ) AS hits,
  STRING_AGG(name, ',' ORDER BY id) AS names
FROM qg_points;
"""

SECRET_PATTERNS = (
    re.compile(r"(?i)(password|secret|token|signature|credential)=([^\s'\"]+)"),
    re.compile(r"(?i)(postgres(?:ql)?://[^:\s]+:)([^@/<\s]+)@"),
    re.compile(r"(?i)(X-Amz-(?:Signature|Credential|Security-Token)=)([^&\s]+)"),
)


class DuckDbResult(NamedTuple):
    status: str
    stdout: str
    stderr: str
    returncode: int


def redact(text: str) -> str:
    redacted = text
    redacted = SECRET_PATTERNS[0].sub(lambda match: f"{match.group(1)}=<redacted>", redacted)
    redacted = SECRET_PATTERNS[1].sub(r"\1<redacted>@", redacted)
    redacted = SECRET_PATTERNS[2].sub(r"\1<redacted>", redacted)
    return redacted


def resolve_binary(duckdb_bin: str) -> str | None:
    if "/" in duckdb_bin:
        path = Path(duckdb_bin)
        return str(path) if path.exists() else None
    return shutil.which(duckdb_bin)


def run_version(binary: str) -> str:
    result = subprocess.run(
        [binary, "--version"],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        details = redact(result.stderr or result.stdout).strip()
        raise RuntimeError(f"duckdb --version failed with {result.returncode}: {details}")
    return redact(result.stdout.strip() or result.stderr.strip() or "unknown")


def run_sql(binary: str, sql: str) -> DuckDbResult:
    result = subprocess.run(
        [binary, "-csv", ":memory:", "-c", sql],
        text=True,
        capture_output=True,
        check=False,
    )
    stdout = redact(result.stdout)
    stderr = redact(result.stderr)
    status = "pass" if result.returncode == 0 else "fail"
    return DuckDbResult(status=status, stdout=stdout, stderr=stderr, returncode=result.returncode)


def render_report(
    *,
    status: str,
    duckdb_bin: str,
    duckdb_version: str | None = None,
    engine: DuckDbResult | None = None,
    attach: DuckDbResult | None = None,
    attach_sql: Path | None = None,
    message: str | None = None,
) -> str:
    body = [
        "# DuckDB engine feasibility probe",
        "",
        f"Status: `{status}`",
        f"DuckDB binary: `{duckdb_bin}`",
    ]
    if duckdb_version:
        body.append(f"DuckDB version: `{duckdb_version}`")
    if message:
        body.extend(["", message])
    if engine:
        body.extend(
            [
                "",
                "## Engine smoke",
                "",
                f"Status: `{engine.status}` returncode={engine.returncode}",
                "",
                "```text",
                engine.stdout.strip(),
                "```",
            ]
        )
        if engine.stderr.strip():
            body.extend(["", "stderr:", "", "```text", engine.stderr.strip(), "```"])
    if attach:
        body.extend(
            [
                "",
                "## Reference attach SQL",
                "",
                f"Attach SQL file: `{attach_sql}`",
                f"Status: `{attach.status}` returncode={attach.returncode}",
                "",
                "```text",
                attach.stdout.strip(),
                "```",
            ]
        )
        if attach.stderr.strip():
            body.extend(["", "stderr:", "", "```text", attach.stderr.strip(), "```"])
    body.append("")
    return "\n".join(body)


def validate_engine_output(result: DuckDbResult) -> bool:
    if result.returncode != 0:
        return False
    return "ducklake_extension_loaded" in result.stdout and "engine_spatial_wkb" in result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb-bin", default="duckdb")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument(
        "--attach-sql",
        type=Path,
        help="optional SQL file that attaches and probes a copied DuckLake catalog/prefix",
    )
    args = parser.parse_args(argv)

    args.out.unlink(missing_ok=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    binary = resolve_binary(args.duckdb_bin)
    if not binary:
        args.out.write_text(
            render_report(
                status="missing_duckdb",
                duckdb_bin=args.duckdb_bin,
                message="Install DuckDB CLI or set `DUCKDB_BIN` before running the engine probe.",
            ),
            encoding="utf-8",
        )
        print(f"duckdb_engine_probe_missing out={args.out}", file=sys.stderr)
        return 2

    try:
        version = run_version(binary)
        engine = run_sql(binary, ENGINE_SQL)
        attach = None
        if args.attach_sql:
            attach = run_sql(binary, args.attach_sql.read_text(encoding="utf-8"))
        ok = validate_engine_output(engine) and (attach is None or attach.returncode == 0)
        args.out.write_text(
            render_report(
                status="pass" if ok else "fail",
                duckdb_bin=binary,
                duckdb_version=version,
                engine=engine,
                attach=attach,
                attach_sql=args.attach_sql,
            ),
            encoding="utf-8",
        )
    except (OSError, RuntimeError) as error:
        args.out.write_text(
            render_report(status="fail", duckdb_bin=binary, message=redact(str(error))),
            encoding="utf-8",
        )
        print(f"duckdb engine probe failed: {error}", file=sys.stderr)
        return 1

    if ok:
        print(f"duckdb_engine_probe_ok out={args.out}")
        return 0
    print(f"duckdb_engine_probe_failed out={args.out}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
