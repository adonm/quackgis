#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import hashlib
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "bootstrap_duckdb", ROOT / "scripts/bootstrap_duckdb.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BootstrapDuckDBTests(unittest.TestCase):
    def test_only_pinned_official_extensions_are_installed(self) -> None:
        self.assertEqual(MODULE.EXTENSIONS, ("ducklake", "spatial"))
        self.assertEqual(
            MODULE.EXTENSION_SHA256["spatial"],
            MODULE.BUNDLE["duckdb"]["artifact"]["official_extension_sha256"]["spatial"],
        )
        self.assertNotEqual(
            MODULE.EXTENSION_SHA256["ducklake"],
            MODULE.BUNDLE["extensions"]["ducklake"]["artifact"]["sha256"],
        )

    def test_cli_is_hashed_before_execution(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            binary = Path(directory) / "duckdb"
            binary.write_bytes(b"pinned test cli")
            expected = hashlib.sha256(binary.read_bytes()).hexdigest()
            with mock.patch.object(MODULE, "CLI_SHA256", expected):
                path, digest = MODULE.require_cli(str(binary))
            self.assertEqual(path, binary.resolve())
            self.assertEqual(digest, expected)
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                MODULE.require_cli(str(binary))

    def test_bootstrap_root_rejects_symlink(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            parent = Path(directory)
            outside = parent / "outside"
            outside.mkdir()
            linked = parent / "linked"
            linked.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                MODULE.require_workspace_root(linked)

    def test_bootstrap_home_rejects_symlink(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            root = Path(directory)
            outside = root / "outside"
            outside.mkdir()
            (root / "home").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                MODULE.prepare_home(root)


if __name__ == "__main__":
    unittest.main()
