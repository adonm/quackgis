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
    "build_native_bundle", ROOT / "scripts/build_native_bundle.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BuildNativeBundleTests(unittest.TestCase):
    def test_normalizes_only_the_exact_spatial_overlay(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            root = Path(directory)
            spatial = root / "spatial"
            (spatial / "vcpkg_ports").mkdir(parents=True)
            manifest = {
                "builtin-baseline": "a" * 40,
                "dependencies": ["roaring", "gdal"],
                "vcpkg-configuration": {
                    "overlay-ports": [str(spatial / "vcpkg_ports")]
                },
            }
            path = root / "vcpkg.json"
            path.write_text(json.dumps(manifest), encoding="utf-8")
            normalized = MODULE.normalized_vcpkg_manifest(path, spatial)
            self.assertEqual(
                normalized["vcpkg-configuration"]["overlay-ports"],
                ["${SPATIAL_SOURCE}/vcpkg_ports"],
            )

    def test_rejects_an_unowned_overlay(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            root = Path(directory)
            spatial = root / "spatial"
            spatial.mkdir()
            path = root / "vcpkg.json"
            path.write_text(
                json.dumps(
                    {
                        "vcpkg-configuration": {
                            "overlay-ports": [str(root / "other")]
                        }
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "unexpected overlay"):
                MODULE.normalized_vcpkg_manifest(path, spatial)

    def test_make_environment_uses_only_prepared_sources(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        sources = ROOT / ".tmp/native-bundle/sources"
        environment = MODULE.make_environment(bundle, sources)
        variables = environment["EXTRA_CMAKE_VARIABLES"]
        self.assertIn(str((sources / "ducklake").resolve()), variables)
        self.assertIn(str((sources / "spatial").resolve()), variables)
        self.assertNotIn("GIT_URL", variables)
        self.assertEqual(environment["OVERRIDE_GIT_DESCRIBE"], "v1.5.4-0-g08e34c447b")


if __name__ == "__main__":
    unittest.main()
