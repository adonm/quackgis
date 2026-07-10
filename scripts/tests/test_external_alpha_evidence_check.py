#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import copy
import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "external_alpha_evidence_check.py"
SPEC = importlib.util.spec_from_file_location("external_alpha_evidence_check", MODULE_PATH)
assert SPEC and SPEC.loader
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


SHA = "a" * 40
DIGEST = "sha256:" + "b" * 64


def manifest(claim: str = "external_alpha_promotion") -> dict[str, object]:
    return {
        "source_sha": SHA,
        "quackgis_image_digest": DIGEST,
        "claim": claim,
        "storage_profile": "postgresql-s3-compatible",
        "providers": {
            "postgresql": {
                "name": "managed-postgres",
                "version": "16",
                "region": "local-test",
                "service_class": "dev",
            },
            "object_store": {
                "name": "s3-compatible",
                "version": "2026-07",
                "region": "local-test",
                "service_class": "dev",
            },
        },
        "dataset": {
            "description": "external alpha fixture",
            "row_count": 3,
            "file_count": 2,
            "object_bytes": 1024,
            "object_prefix": "s3://bucket/quackgis-alpha/redacted",
        },
        "commands": [
            "EXTERNAL_QUACKGIS_CATALOG_URL=postgres://user:<redacted>@db.example/quackgis just kind-external-alpha-smoke"
        ],
        "drills": {
            drill: {"status": "pass", "evidence": f"{drill}.log"}
            for drill in CHECK.REQUIRED_DRILLS
        },
        "artifacts": {
            "compatibility_report": ".tmp/compatibility/README.md",
            "metrics": ".tmp/compatibility/metrics.json",
            "dashboard": ".tmp/compatibility/metrics-dashboard.md",
            "logs": ".tmp/compatibility/*.log",
        },
    }


def metrics() -> dict[str, object]:
    return {
        "run": {"github_sha": SHA},
        "summary": {"passed": 1, "failed": 0, "not_run": 0},
        "checks": {
            "external_lake_probe": {
                "label": "External profile storage",
                "status": "pass",
                "metrics": {"native_mutation_aborts": 0},
            }
        },
    }


class ExternalAlphaEvidenceCheckTests(unittest.TestCase):
    def test_full_pass_manifest_accepts_alpha_promotion(self) -> None:
        summary = CHECK.validate_manifest(manifest())
        CHECK.validate_metrics(metrics(), summary)
        self.assertEqual(summary["claim"], "external_alpha_promotion")
        self.assertTrue(CHECK.render(summary).startswith("# External-service Alpha"))

    def test_skipped_drill_must_be_wiring_smoke(self) -> None:
        packet = manifest()
        packet["drills"]["backup_restore"]["status"] = "skip"  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)
        packet["claim"] = "external_wiring_smoke"
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["statuses"]["backup_restore"], "skip")

    def test_unredacted_secrets_and_metric_mismatch_fail_closed(self) -> None:
        packet = manifest()
        packet["commands"] = ["EXTERNAL_QUACKGIS_CATALOG_URL=postgres://user:secret@db/quackgis just kind-external-alpha-smoke"]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)
        packet["commands"] = ["password=<redacted> EXTERNAL_QUACKGIS_S3_SECRET_ACCESS_KEY=plain-secret just kind-external-alpha-smoke"]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        summary = CHECK.validate_manifest(manifest())
        bad_metrics = copy.deepcopy(metrics())
        bad_metrics["checks"]["external_lake_probe"]["status"] = "fail"  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_metrics(bad_metrics, summary)

    def test_cli_removes_stale_output_on_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            manifest_path = temp / "manifest.json"
            metrics_path = temp / "metrics.json"
            out = temp / "README.md"
            manifest_path.write_text(json.dumps({"bad": True}), encoding="utf-8")
            metrics_path.write_text(json.dumps(metrics()), encoding="utf-8")
            out.write_text("stale", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                status = CHECK.main(
                    ["--manifest", str(manifest_path), "--metrics", str(metrics_path), "--out", str(out)]
                )
            self.assertEqual(status, 1)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
