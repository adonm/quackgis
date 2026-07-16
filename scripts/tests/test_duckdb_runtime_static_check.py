#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "duckdb_runtime_static_check", ROOT / "scripts/duckdb_runtime_static_check.py"
)
assert SPEC and SPEC.loader
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class DuckDbRuntimeStaticCheckTests(unittest.TestCase):
    def test_repository_containerfile_passes(self) -> None:
        self.assertEqual(CHECK.validate(ROOT / "deploy/Containerfile.duckdb-runtime"), [])

    def test_online_install_is_rejected(self) -> None:
        original = (ROOT / "deploy/Containerfile.duckdb-runtime").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Containerfile"
            path.write_text(original + "\nRUN duckdb -c 'INSTALL spatial'\n", encoding="utf-8")
            self.assertTrue(any("online-install" in error for error in CHECK.validate(path)))

    def test_pinned_ducklake_digest_drift_is_rejected(self) -> None:
        original = (ROOT / "deploy/Containerfile.duckdb-runtime").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Containerfile"
            path.write_text(
                original.replace(
                    CHECK.PINNED_DUCKLAKE_SHA256,
                    "0" * 64,
                ),
                encoding="utf-8",
            )
            self.assertTrue(any("SHA256" in error for error in CHECK.validate(path)))


if __name__ == "__main__":
    unittest.main()
