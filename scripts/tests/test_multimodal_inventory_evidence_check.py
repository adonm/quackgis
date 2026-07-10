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
MODULE_PATH = ROOT / "scripts" / "multimodal_inventory_evidence_check.py"
SPEC = importlib.util.spec_from_file_location("multimodal_inventory_evidence_check", MODULE_PATH)
assert SPEC and SPEC.loader
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


SHA = "a" * 40


def collection(family: str, name: str) -> dict[str, object]:
    return {
        "family": family,
        "name": name,
        "status": "pass",
        "object_prefix": f"s3://bucket/copied/{name}",
        "row_count": 10,
        "object_count": 3,
        "object_bytes": 4096,
        "uri_policy": "non_secret_stable_uris",
        "checksum_evidence": f"{name}/checksums.log",
        "crs_epoch_evidence": f"{name}/crs.log",
        "footprint_evidence": f"{name}/footprints.log",
        "lifecycle_evidence": f"{name}/lifecycle.log",
        "restore_evidence": f"{name}/restore.log",
        "workload_evidence": f"{name}/workload.log",
    }


def manifest(claim: str = "multimodal_inventory_promotion") -> dict[str, object]:
    return {
        "source_sha": SHA,
        "claim": claim,
        "storage_profile": "postgresql-s3-compatible",
        "vector_gate_evidence": "compatibility/README.md",
        "collections": [
            collection("cog_raster", "city-cog"),
            collection("copc_laz_pointcloud", "city-copc"),
        ],
    }


class MultimodalInventoryEvidenceCheckTests(unittest.TestCase):
    def test_promotion_requires_cog_and_copc_collections(self) -> None:
        summary = CHECK.validate_manifest(manifest())
        self.assertEqual(summary["passed"], 2)
        self.assertEqual(summary["rows"], 20)
        self.assertIn("city-cog", CHECK.render(summary))

        packet = manifest()
        packet["collections"] = [collection("cog_raster", "city-cog")]  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

    def test_wiring_allows_skip_but_promotion_does_not(self) -> None:
        packet = manifest("multimodal_inventory_wiring")
        packet["collections"][1]["status"] = "skip"  # type: ignore[index]
        summary = CHECK.validate_manifest(packet)
        self.assertEqual(summary["skipped"], 1)

        packet["claim"] = "multimodal_inventory_promotion"
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

    def test_secret_uris_and_missing_evidence_fail_closed(self) -> None:
        packet = manifest()
        packet["collections"][0]["object_prefix"] = "s3://bucket/copied?X-Amz-Signature=abc"  # type: ignore[index]
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.validate_manifest(packet)

        packet = manifest()
        del packet["collections"][0]["checksum_evidence"]  # type: ignore[index]
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
