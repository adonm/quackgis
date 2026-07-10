#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate a DuckDB/DuckLake reference-reader evidence manifest.

This is a static compatibility gate. It does not run DuckDB itself; it ensures a
packet cannot claim DuckDB reference readability without naming the DuckDB and
DuckLake extension versions, storage profile, copied catalog/object source, read
parity evidence, mutation/compaction evidence, and migration implication when the
result remains non-standard.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


VALID_CLAIMS = {"duckdb_reference_wiring", "duckdb_reference_readable"}
VALID_DECISIONS = {
    "reference_readable",
    "non_standard",
    "export_required",
    "writer_candidate",
}
VALID_CONNECTION_PATHS = {"cli", "adbc", "other"}
VALID_STATUS = {"pass", "fail", "skip"}
VALID_STORAGE_PROFILES = {
    "local-sqlite",
    "duckdb-local-ducklake",
    "postgresql-s3-compatible",
}
REQUIRED_CHECKS = (
    "attach",
    "schema",
    "read_samples",
    "spatial_bytes",
    "mutation_compaction",
    "unsupported_metadata",
)
OPTIONAL_CHECKS = ("snapshot_time_travel",)
ALL_CHECKS = REQUIRED_CHECKS + OPTIONAL_CHECKS
SECRET_PATTERNS = (
    re.compile(r"(?i)(password|secret|token|signature|credential)=([^\s'\"]+)"),
    re.compile(r"(?i)(postgres(?:ql)?://[^:\s]+:)([^@/<\s]+)@"),
    re.compile(r"(?i)X-Amz-(Signature|Credential|Security-Token)="),
)


class EvidenceError(ValueError):
    """The evidence packet is malformed or overclaims."""


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be an object")
    return value


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise EvidenceError(f"{label} must be a non-empty string")
    return value


def require_int(value: Any, label: str, *, positive: bool = False) -> int:
    if isinstance(value, bool):
        raise EvidenceError(f"{label} must be an integer")
    try:
        parsed = int(value)
    except (TypeError, ValueError) as error:
        raise EvidenceError(f"{label} must be an integer") from error
    if str(value) != str(parsed):
        raise EvidenceError(f"{label} must be an integer")
    if parsed < 0 or (positive and parsed == 0):
        qualifier = "positive" if positive else "non-negative"
        raise EvidenceError(f"{label} must be {qualifier}")
    return parsed


def assert_no_secrets(text: str, label: str) -> None:
    for pattern in SECRET_PATTERNS:
        if pattern.search(text):
            raise EvidenceError(f"{label} appears to contain an unredacted secret")


def load_json(path: Path) -> dict[str, Any]:
    try:
        return require_object(json.loads(path.read_text(encoding="utf-8")), "manifest")
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"could not read manifest {path}: {error}") from error


def validate_reader(manifest: dict[str, Any]) -> dict[str, str]:
    reader = require_object(manifest.get("reader"), "reader")
    duckdb_version = require_string(reader.get("duckdb_version"), "reader.duckdb_version")
    ducklake_extension = require_string(
        reader.get("ducklake_extension"), "reader.ducklake_extension"
    )
    connection_path = require_string(reader.get("connection_path"), "reader.connection_path")
    if connection_path not in VALID_CONNECTION_PATHS:
        raise EvidenceError(f"reader.connection_path must be one of {sorted(VALID_CONNECTION_PATHS)}")
    command_evidence = require_string(reader.get("command_evidence"), "reader.command_evidence")
    assert_no_secrets(command_evidence, "reader.command_evidence")
    return {
        "duckdb_version": duckdb_version,
        "ducklake_extension": ducklake_extension,
        "connection_path": connection_path,
    }


def validate_dataset(manifest: dict[str, Any]) -> dict[str, int | str]:
    dataset = require_object(manifest.get("dataset"), "dataset")
    catalog_source = require_string(dataset.get("catalog_source"), "dataset.catalog_source")
    object_prefix = require_string(dataset.get("object_prefix"), "dataset.object_prefix")
    assert_no_secrets(catalog_source, "dataset.catalog_source")
    assert_no_secrets(object_prefix, "dataset.object_prefix")
    return {
        "description": require_string(dataset.get("description"), "dataset.description"),
        "catalog_source": catalog_source,
        "object_prefix": object_prefix,
        "row_count": require_int(dataset.get("row_count"), "dataset.row_count"),
        "file_count": require_int(dataset.get("file_count"), "dataset.file_count"),
        "object_bytes": require_int(dataset.get("object_bytes"), "dataset.object_bytes", positive=True),
    }


def validate_check(raw: Any, label: str) -> str:
    detail = require_object(raw, label)
    status = require_string(detail.get("status"), f"{label}.status")
    if status not in VALID_STATUS:
        raise EvidenceError(f"{label}.status must be one of {sorted(VALID_STATUS)}")
    require_string(detail.get("evidence"), f"{label}.evidence")
    if status == "skip":
        require_string(detail.get("skip_reason"), f"{label}.skip_reason")
    return status


def validate_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    source_sha = require_string(manifest.get("source_sha"), "source_sha")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise EvidenceError("source_sha must be a 40-character lowercase Git SHA")
    claim = require_string(manifest.get("claim"), "claim")
    if claim not in VALID_CLAIMS:
        raise EvidenceError(f"claim must be one of {sorted(VALID_CLAIMS)}")
    storage_profile = require_string(manifest.get("storage_profile"), "storage_profile")
    if storage_profile not in VALID_STORAGE_PROFILES:
        raise EvidenceError(f"storage_profile must be one of {sorted(VALID_STORAGE_PROFILES)}")

    reader = validate_reader(manifest)
    dataset = validate_dataset(manifest)

    checks = require_object(manifest.get("checks"), "checks")
    statuses: dict[str, str] = {}
    for check in REQUIRED_CHECKS:
        statuses[check] = validate_check(checks.get(check), f"checks.{check}")
    for check in OPTIONAL_CHECKS:
        statuses[check] = validate_check(checks.get(check), f"checks.{check}")
    unknown = set(checks) - set(ALL_CHECKS)
    if unknown:
        raise EvidenceError(f"unknown check keys: {', '.join(sorted(unknown))}")

    decision = require_string(manifest.get("decision"), "decision")
    if decision not in VALID_DECISIONS:
        raise EvidenceError(f"decision must be one of {sorted(VALID_DECISIONS)}")
    if decision in {"non_standard", "export_required"}:
        require_string(manifest.get("migration_implication"), "migration_implication")
    incomplete_required = [check for check in REQUIRED_CHECKS if statuses[check] != "pass"]
    if decision == "reference_readable" and incomplete_required:
        raise EvidenceError(
            "decision=reference_readable requires required checks to pass: "
            + ", ".join(incomplete_required)
        )
    if claim == "duckdb_reference_readable":
        if decision != "reference_readable":
            raise EvidenceError("duckdb_reference_readable requires decision=reference_readable")

    return {
        "claim": claim,
        "source_sha": source_sha,
        "storage_profile": storage_profile,
        "reader": reader,
        "dataset": dataset,
        "statuses": statuses,
        "decision": decision,
    }


def render(summary: dict[str, Any]) -> str:
    passed = sum(1 for status in summary["statuses"].values() if status == "pass")
    failed = sum(1 for status in summary["statuses"].values() if status == "fail")
    skipped = sum(1 for status in summary["statuses"].values() if status == "skip")
    body = [
        "# DuckDB reference-reader evidence check",
        "",
        f"Claim: `{summary['claim']}`",
        f"Source SHA: `{summary['source_sha']}`",
        f"Storage profile: `{summary['storage_profile']}`",
        f"DuckDB: `{summary['reader']['duckdb_version']}`",
        f"DuckLake extension: `{summary['reader']['ducklake_extension']}`",
        f"Connection path: `{summary['reader']['connection_path']}`",
        (
            "Dataset: "
            f"rows={summary['dataset']['row_count']} files={summary['dataset']['file_count']} "
            f"object_bytes={summary['dataset']['object_bytes']}"
        ),
        f"Decision: `{summary['decision']}`",
        f"Checks: {passed} passed, {failed} failed, {skipped} skipped",
        "",
        "| Check | Status |",
        "|---|---|",
    ]
    for check in ALL_CHECKS:
        body.append(f"| `{check}` | {summary['statuses'][check]} |")
    body.append("")
    return "\n".join(body)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        args.out.unlink(missing_ok=True)
        summary = validate_manifest(load_json(args.manifest))
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render(summary), encoding="utf-8")
    except (EvidenceError, OSError) as error:
        print(f"duckdb reference evidence check failed: {error}", file=sys.stderr)
        return 1
    print(f"duckdb_reference_evidence_check_ok out={args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
