#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "duckdb_reference_evidence_check.py"
SPEC = importlib.util.spec_from_file_location("duckdb_reference_evidence_check", MODULE_PATH)
assert SPEC and SPEC.loader
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


SHA = "a" * 40


def passed_check(evidence: str) -> dict[str, str]:
    return {"status": "pass", "evidence": evidence}


def manifest(claim: str = "duckdb_reference_readable") -> dict[str, object]:
    return {
        "source_sha": SHA,
        "claim": claim,
        "storage_profile": "postgresql-s3-compatible",
        "reader": {
            "duckdb_version": "1.5.0",
            "ducklake_extension": "ducklake bundled 2026-07",
            "connection_path": "adbc",
            "command_evidence": "interop/duckdb-adbc.log",
        },
        "dataset": {
            "description": "copied external alpha fixture",
            "catalog_source": "postgres://user:<redacted>@db.example/quackgis_alpha_copy",
            "object_prefix": "s3://bucket/quackgis-alpha-copy",
            "row_count": 3,
            "file_count": 2,
            "object_bytes": 1024,
        },
        "checks": {
            check: passed_check(f"interop/{check}.log") for check in CHECK.ALL_CHECKS
        },
        "decision": "reference_readable",
    }


class DuckDbReferenceEvidenceCheckTests(unittest.TestCase):
    def test_reference_readable_manifest_accepts_all_required_checks(self) -> None:
        summary = CHECK.validate_manifest(manifest())
        self.assertEqual(summary["claim"], "duckdb_reference_readable")
        self.assertEqual(summary["decision"], "reference_readable")
        self.assertIn("DuckDB reference-reader", CHECK.render(summary))

    def test_duckdb_authored_local_profile_can_claim_reference_readable(self) -> None:
        packet = manifest()
        packet["storage_profile"] = "duckdb-local-ducklake"
        packet["dataset"]["catalog_source"] = ".tmp/duckdb-authority/catalog.ducklake"  # type: ignore[index]
        packet["dataset"]["object_prefix"] = ".tmp/duckdb-authority/data"  # type: ignore[index]
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["storage_profile"], "duckdb-local-ducklake")

    def test_reference_readable_allows_optional_snapshot_skip_with_reason(self) -> None:
        packet = manifest()
        packet["checks"]["snapshot_time_travel"] = {  # type: ignore[index]
            "status": "skip",
            "evidence": "interop/snapshot-not-supported.log",
            "skip_reason": "DuckDB extension version did not expose comparable time travel",
        }
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["statuses"]["snapshot_time_travel"], "skip")

    def test_reference_readable_requires_required_checks_and_decision(self) -> None:
        packet = manifest()
        packet["checks"]["schema"] = {  # type: ignore[index]
            "status": "skip",
            "evidence": "interop/schema.log",
            "skip_reason": "not implemented",
        }
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        packet = manifest()
        packet["decision"] = "non_standard"
        packet["migration_implication"] = "export before standard-reader claims"
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

    def test_wiring_non_standard_requires_migration_implication(self) -> None:
        packet = manifest("duckdb_reference_wiring")
        packet["decision"] = "non_standard"
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        packet["migration_implication"] = "export or migrate catalog metadata"
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["decision"], "non_standard")

    def test_reference_readable_decision_requires_required_checks(self) -> None:
        packet = manifest("duckdb_reference_wiring")
        packet["checks"]["attach"] = {  # type: ignore[index]
            "status": "skip",
            "evidence": "interop/attach.log",
            "skip_reason": "not run",
        }
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        packet = manifest("duckdb_reference_wiring")
        packet["checks"]["attach"] = {"status": "fail", "evidence": "interop/attach.log"}  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

    def test_wiring_can_record_failed_attach_when_decision_is_non_standard(self) -> None:
        packet = manifest("duckdb_reference_wiring")
        packet["checks"]["attach"] = {"status": "fail", "evidence": "interop/attach.log"}  # type: ignore[index]
        packet["decision"] = "non_standard"
        packet["migration_implication"] = "add allocator/export compatibility before DuckDB claims"
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["statuses"]["attach"], "fail")

    def test_secret_inputs_and_unknown_checks_fail_closed(self) -> None:
        packet = manifest()
        packet["reader"]["command_evidence"] = "password=plain-secret duckdb"  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        packet = manifest()
        packet["checks"]["extra"] = passed_check("extra.log")  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

    def test_cli_removes_stale_output_on_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            manifest_path = temp / "manifest.json"
            out = temp / "README.md"
            manifest_path.write_text(json.dumps({"bad": True}), encoding="utf-8")
            out.write_text("stale", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                status = CHECK.main(["--manifest", str(manifest_path), "--out", str(out)])
            self.assertEqual(status, 1)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
