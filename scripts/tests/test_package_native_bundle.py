#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "package_native_bundle", ROOT / "scripts/package_native_bundle.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PackageNativeBundleTests(unittest.TestCase):
    def test_metadata_is_deterministic_and_path_free(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            out = Path(directory)
            paths = MODULE.write_metadata(bundle, out)
            first = {name: path.read_bytes() for name, path in paths.items()}
            MODULE.write_metadata(bundle, out)
            second = {name: path.read_bytes() for name, path in paths.items()}
            self.assertEqual(first, second)
            self.assertNotIn(str(ROOT).encode(), b"".join(first.values()))

    def test_spdx_describes_exact_selected_artifacts(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        document = MODULE.spdx_document(bundle)
        self.assertEqual(document["spdxVersion"], "SPDX-2.3")
        packages = {package["name"]: package for package in document["packages"]}
        self.assertEqual(
            packages["DuckDB"]["checksums"][0]["checksumValue"],
            bundle["duckdb"]["artifact"]["library_sha256"],
        )
        self.assertEqual(
            packages["DuckLake"]["checksums"][0]["checksumValue"],
            bundle["extensions"]["ducklake"]["artifact"]["sha256"],
        )
        self.assertEqual(len(document["relationships"]), 3)

    def test_incomplete_spatial_licenses_remain_release_blocking(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        inventory = MODULE.license_inventory(bundle)
        self.assertFalse(inventory["complete"])
        self.assertEqual(
            [item["component"] for item in inventory["unresolved"]],
            bundle["extensions"]["spatial"]["bundled_dependencies"],
        )
        self.assertTrue(all(item["release_blocking"] for item in inventory["unresolved"]))

    def test_generated_files_are_valid_json(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            paths = MODULE.write_metadata(bundle, Path(directory))
            for path in paths.values():
                self.assertIsInstance(json.loads(path.read_text(encoding="utf-8")), dict)


if __name__ == "__main__":
    unittest.main()
