#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "build_pinned_ducklake", ROOT / "scripts/build_pinned_ducklake.py"
)
assert SPEC and SPEC.loader
BUILD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD)


class PinnedDuckLakeBuildTests(unittest.TestCase):
    def test_repository_pin_and_patch_are_consistent(self) -> None:
        pin = BUILD.load_pin()
        self.assertEqual(pin["schema_version"], 1)
        self.assertEqual(
            BUILD.file_sha256(ROOT / str(pin["patch"])), pin["patch_sha256"]
        )

    def test_pin_rejects_artifact_digest_drift(self) -> None:
        pin = BUILD.load_pin().copy()
        pin["artifact_sha256"] = "not-a-digest"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pin.json"
            path.write_text(json.dumps(pin), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "artifact_sha256"):
                BUILD.load_pin(path)


if __name__ == "__main__":
    unittest.main()
