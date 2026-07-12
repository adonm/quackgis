#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Unit tests for the common evidence envelope validator."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from evidence_manifest_check import validate  # noqa: E402


def valid_manifest() -> dict[str, object]:
    digest = "a" * 64
    return {
        "schema_version": 1,
        "profile_id": "profile-v1",
        "evidence_level": "smoke",
        "execution_environment": "host_process",
        "status": "pass",
        "source": {
            "sha": "b" * 40,
            "dirty": False,
            "status_sha256": None,
            "diff_sha256": None,
        },
        "runtime": {
            "duckdb_version": "1.5.4",
            "platform": "linux-amd64",
            "libduckdb_sha256": digest,
            "extensions": {"ducklake": digest, "spatial": digest},
        },
        "host": {"logical_cpus": 8, "storage": "local NVMe"},
        "data": {},
        "correctness": {},
        "measurements": {},
        "budgets": {},
        "scope": "test",
    }


class EvidenceManifestCheckTests(unittest.TestCase):
    def test_accepts_complete_smoke_envelope(self) -> None:
        self.assertEqual(validate(valid_manifest()), [])

    def test_reference_requires_clean_source_and_storage(self) -> None:
        manifest = valid_manifest()
        manifest["evidence_level"] = "reference"
        manifest["source"]["dirty"] = True  # type: ignore[index]
        manifest["source"]["status_sha256"] = "c" * 64  # type: ignore[index]
        manifest["source"]["diff_sha256"] = "d" * 64  # type: ignore[index]
        manifest["host"]["storage"] = "unspecified"  # type: ignore[index]
        errors = validate(manifest)
        self.assertTrue(any("clean source" in error for error in errors))
        self.assertTrue(any("host.storage" in error for error in errors))

    def test_rejects_runtime_paths_and_missing_digest(self) -> None:
        manifest = valid_manifest()
        manifest["runtime"]["path"] = "/secret"  # type: ignore[index]
        del manifest["runtime"]["extensions"]["spatial"]  # type: ignore[index]
        errors = validate(manifest)
        self.assertTrue(any("path" in error for error in errors))
        self.assertTrue(any("spatial" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
