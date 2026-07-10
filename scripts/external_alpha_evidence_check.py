#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate an external-service Alpha evidence packet manifest.

This is a static artifact gate: it does not run the managed PostgreSQL/S3 drills,
but it prevents a partial wiring smoke from being mislabeled as external Alpha
promotion evidence.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


REQUIRED_DRILLS = (
    "static_preflight",
    "emulator_preflight",
    "real_service_wiring",
    "multi_pod_readers_writers",
    "backup_restore",
    "credential_rotation",
    "catalog_restart",
    "object_store_latency_throttling",
    "failed_writer_cleanup",
    "catalog_refresh_visibility",
)
VALID_STATUS = {"pass", "fail", "skip"}
VALID_CLAIMS = {"external_wiring_smoke", "external_alpha_promotion"}
VALID_INTEROP_RESULTS = {
    "standard_ducklake_readable",
    "quackgis_multicatalog_non_standard",
    "not_tested",
}
SECRET_PATTERNS = (
    re.compile(r"(?i)(password|secret|access[_-]?key|token)=([^\s'\"]+)"),
    re.compile(r"(?i)postgres://[^:\s]+:[^@\s]+@"),
    re.compile(r"(?i)x-amz-signature="),
)


class EvidenceError(ValueError):
    """The evidence packet is malformed or overclaims."""


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be an object")
    return value


def require_string(value: Any, label: str, *, non_empty: bool = True) -> str:
    if not isinstance(value, str):
        raise EvidenceError(f"{label} must be a string")
    if non_empty and not value.strip():
        raise EvidenceError(f"{label} must not be empty")
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


def optional_int(value: Any, label: str) -> int | None:
    if value is None:
        return None
    return require_int(value, label)


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        return require_object(json.loads(path.read_text(encoding="utf-8")), label)
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"could not read {label} {path}: {error}") from error


def assert_redacted(text: str, label: str) -> None:
    for pattern in SECRET_PATTERNS:
        for match in pattern.finditer(text):
            matched = match.group(0).lower()
            if "..." in matched or "redacted" in matched:
                continue
            raise EvidenceError(f"{label} appears to contain an unredacted secret: {match.group(0)!r}")


def validate_backup_restore_drill(detail: dict[str, Any]) -> dict[str, int]:
    rpo_seconds = require_int(detail.get("rpo_seconds"), "drills.backup_restore.rpo_seconds")
    rto_seconds = require_int(detail.get("rto_seconds"), "drills.backup_restore.rto_seconds")
    require_string(detail.get("restored_catalog"), "drills.backup_restore.restored_catalog")
    require_string(detail.get("restored_object_prefix"), "drills.backup_restore.restored_object_prefix")
    require_string(detail.get("read_smoke_evidence"), "drills.backup_restore.read_smoke_evidence")
    return {"rpo_seconds": rpo_seconds, "rto_seconds": rto_seconds}


def validate_failed_writer_cleanup_drill(detail: dict[str, Any]) -> dict[str, int]:
    candidates = require_int(
        detail.get("orphan_candidates"), "drills.failed_writer_cleanup.orphan_candidates"
    )
    quarantined = require_int(
        detail.get("quarantined_candidates"),
        "drills.failed_writer_cleanup.quarantined_candidates",
    )
    deleted = optional_int(
        detail.get("deleted_from_quarantine"),
        "drills.failed_writer_cleanup.deleted_from_quarantine",
    )
    if quarantined > candidates:
        raise EvidenceError(
            "drills.failed_writer_cleanup.quarantined_candidates cannot exceed orphan_candidates"
        )
    if deleted is not None and deleted > quarantined:
        raise EvidenceError(
            "drills.failed_writer_cleanup.deleted_from_quarantine cannot exceed quarantined_candidates"
        )
    require_string(detail.get("quarantine_prefix"), "drills.failed_writer_cleanup.quarantine_prefix")
    require_string(detail.get("representative_reads"), "drills.failed_writer_cleanup.representative_reads")
    return {"orphan_candidates": candidates, "quarantined_candidates": quarantined}


def validate_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    source_sha = require_string(manifest.get("source_sha"), "source_sha")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise EvidenceError("source_sha must be a 40-character lowercase Git SHA")
    digest = require_string(manifest.get("quackgis_image_digest"), "quackgis_image_digest")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        raise EvidenceError("quackgis_image_digest must be a sha256 digest")
    claim = require_string(manifest.get("claim"), "claim")
    if claim not in VALID_CLAIMS:
        raise EvidenceError(f"claim must be one of {sorted(VALID_CLAIMS)}")
    if require_string(manifest.get("storage_profile"), "storage_profile") != "postgresql-s3-compatible":
        raise EvidenceError("storage_profile must be postgresql-s3-compatible")

    providers = require_object(manifest.get("providers"), "providers")
    for provider in ("postgresql", "object_store"):
        provider_info = require_object(providers.get(provider), f"providers.{provider}")
        for field in ("name", "version", "region", "service_class"):
            require_string(provider_info.get(field), f"providers.{provider}.{field}")

    dataset = require_object(manifest.get("dataset"), "dataset")
    for field in ("description", "object_prefix"):
        require_string(dataset.get(field), f"dataset.{field}")
    rows = require_int(dataset.get("row_count"), "dataset.row_count")
    files = require_int(dataset.get("file_count"), "dataset.file_count")
    object_bytes = require_int(dataset.get("object_bytes"), "dataset.object_bytes", positive=True)

    interop = require_object(manifest.get("catalog_interoperability"), "catalog_interoperability")
    interop_result = require_string(interop.get("result"), "catalog_interoperability.result")
    if interop_result not in VALID_INTEROP_RESULTS:
        raise EvidenceError(
            f"catalog_interoperability.result must be one of {sorted(VALID_INTEROP_RESULTS)}"
        )
    require_string(interop.get("standard_reader"), "catalog_interoperability.standard_reader")
    require_string(interop.get("evidence"), "catalog_interoperability.evidence")
    if claim == "external_alpha_promotion" and interop_result == "not_tested":
        raise EvidenceError(
            "external_alpha_promotion requires an explicit standard/non-standard catalog interoperability result"
        )
    if interop_result == "quackgis_multicatalog_non_standard":
        require_string(
            interop.get("migration_implication"),
            "catalog_interoperability.migration_implication",
        )

    commands = manifest.get("commands")
    if not isinstance(commands, list) or not commands:
        raise EvidenceError("commands must be a non-empty list")
    for idx, command in enumerate(commands):
        assert_redacted(require_string(command, f"commands[{idx}]"), f"commands[{idx}]")

    drills = require_object(manifest.get("drills"), "drills")
    statuses: dict[str, str] = {}
    backup_restore: dict[str, int] | None = None
    failed_writer_cleanup: dict[str, int] | None = None
    for drill in REQUIRED_DRILLS:
        detail = require_object(drills.get(drill), f"drills.{drill}")
        status = require_string(detail.get("status"), f"drills.{drill}.status")
        if status not in VALID_STATUS:
            raise EvidenceError(f"drills.{drill}.status must be one of {sorted(VALID_STATUS)}")
        require_string(detail.get("evidence"), f"drills.{drill}.evidence")
        if status == "pass" and drill == "backup_restore":
            backup_restore = validate_backup_restore_drill(detail)
        if status == "pass" and drill == "failed_writer_cleanup":
            failed_writer_cleanup = validate_failed_writer_cleanup_drill(detail)
        statuses[drill] = status
    unknown = set(drills) - set(REQUIRED_DRILLS)
    if unknown:
        raise EvidenceError(f"unknown drill keys: {', '.join(sorted(unknown))}")
    if any(status == "fail" for status in statuses.values()):
        raise EvidenceError("failed drills cannot be accepted as an external Alpha packet")
    if claim == "external_alpha_promotion" and any(status == "skip" for status in statuses.values()):
        raise EvidenceError("external_alpha_promotion requires every drill to pass; use external_wiring_smoke when any drill is skipped")
    if claim == "external_wiring_smoke" and all(status == "pass" for status in statuses.values()):
        raise EvidenceError("external_wiring_smoke underclaims a full-pass packet; use external_alpha_promotion")

    artifacts = require_object(manifest.get("artifacts"), "artifacts")
    for field in ("compatibility_report", "metrics", "dashboard", "logs"):
        require_string(artifacts.get(field), f"artifacts.{field}")
    return {
        "claim": claim,
        "source_sha": source_sha,
        "rows": rows,
        "files": files,
        "object_bytes": object_bytes,
        "catalog_interoperability": interop_result,
        "statuses": statuses,
        "backup_restore": backup_restore,
        "failed_writer_cleanup": failed_writer_cleanup,
    }


def validate_metrics(metrics: dict[str, Any], manifest_summary: dict[str, Any]) -> None:
    run = require_object(metrics.get("run"), "metrics.run")
    github_sha = run.get("github_sha")
    if github_sha and github_sha != manifest_summary["source_sha"]:
        raise EvidenceError("metrics.run.github_sha does not match manifest source_sha")
    checks = require_object(metrics.get("checks"), "metrics.checks")
    external = require_object(checks.get("external_lake_probe"), "metrics.checks.external_lake_probe")
    status = require_string(external.get("status"), "metrics.checks.external_lake_probe.status")
    if manifest_summary["statuses"]["real_service_wiring"] == "pass" and status != "pass":
        raise EvidenceError("real_service_wiring passed in manifest but external_lake_probe metrics did not pass")


def render(summary: dict[str, Any]) -> str:
    passed = sum(1 for status in summary["statuses"].values() if status == "pass")
    skipped = sum(1 for status in summary["statuses"].values() if status == "skip")
    body = [
        "# External-service Alpha evidence check",
        "",
        f"Claim: `{summary['claim']}`",
        f"Source SHA: `{summary['source_sha']}`",
        f"Dataset: rows={summary['rows']} files={summary['files']} object_bytes={summary['object_bytes']}",
        f"Catalog interoperability: `{summary['catalog_interoperability']}`",
        f"Drills: {passed} passed, {skipped} skipped, 0 failed",
        "",
    ]
    if summary.get("backup_restore"):
        backup = summary["backup_restore"]
        body.extend(
            [
                f"Backup/restore: RPO={backup['rpo_seconds']}s RTO={backup['rto_seconds']}s",
                "",
            ]
        )
    if summary.get("failed_writer_cleanup"):
        cleanup = summary["failed_writer_cleanup"]
        body.extend(
            [
                "Failed-writer cleanup: "
                f"orphan_candidates={cleanup['orphan_candidates']} "
                f"quarantined_candidates={cleanup['quarantined_candidates']}",
                "",
            ]
        )
    body.extend(["| Drill | Status |", "|---|---|"])
    body.extend(f"| `{drill}` | {status} |" for drill, status in summary["statuses"].items())
    body.append("")
    return "\n".join(body)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--metrics", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        args.out.unlink(missing_ok=True)
        manifest_summary = validate_manifest(load_json(args.manifest, "manifest"))
        validate_metrics(load_json(args.metrics, "metrics"), manifest_summary)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render(manifest_summary), encoding="utf-8")
    except (EvidenceError, OSError) as error:
        print(f"external alpha evidence check failed: {error}", file=sys.stderr)
        return 1
    print(f"external_alpha_evidence_check_ok out={args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
