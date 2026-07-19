#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("native_bundle", ROOT / "scripts/native_bundle.py")
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativeBundleTests(unittest.TestCase):
    def test_repository_bundle_is_complete(self) -> None:
        bundle = MODULE.load_bundle()
        self.assertEqual(bundle["duckdb"]["version"], "1.5.4")
        self.assertEqual(
            bundle["extensions"]["ducklake"]["duckdb_commit"],
            bundle["duckdb"]["source"]["commit"],
        )

    def test_extension_core_mismatch_is_rejected(self) -> None:
        bundle = copy.deepcopy(MODULE.load_bundle())
        bundle["extensions"]["spatial"]["duckdb_commit"] = "0" * 40
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as output:
            json.dump(bundle, output)
            path = Path(output.name)
        try:
            with self.assertRaisesRegex(ValueError, "different DuckDB commit"):
                MODULE.load_bundle(path, ROOT)
        finally:
            path.unlink()

    def test_patch_digest_drift_is_rejected(self) -> None:
        bundle = MODULE.load_bundle()
        original = ROOT / bundle["extensions"]["ducklake"]["patch_series"]
        series = json.loads(original.read_text(encoding="utf-8"))
        series["patches"][0]["sha256"] = "0" * 64
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            temp_root = Path(directory)
            relative = Path("patches/ducklake/series.json")
            (temp_root / relative.parent).mkdir(parents=True)
            (temp_root / relative).write_text(json.dumps(series), encoding="utf-8")
            (temp_root / "patches/ducklake/ducklake-column-info.patch").write_bytes(
                (ROOT / "patches/ducklake/ducklake-column-info.patch").read_bytes()
            )
            with self.assertRaisesRegex(ValueError, "checksum drifted"):
                MODULE.validate_series(bundle, "ducklake", temp_root)

    def test_bundle_digest_is_order_independent(self) -> None:
        bundle = MODULE.load_bundle()
        reversed_bundle = dict(reversed(list(bundle.items())))
        self.assertEqual(
            MODULE.canonical_sha256(bundle), MODULE.canonical_sha256(reversed_bundle)
        )

    def test_every_patch_has_an_upstream_deletion_review(self) -> None:
        bundle = MODULE.load_bundle()
        review = MODULE.validate_upstream_review(bundle)
        reviewed = {item["path"] for item in review["patch_reviews"]}
        tracked = {
            patch["path"]
            for component in MODULE.COMPONENTS
            for patch in MODULE.validate_series(bundle, component, ROOT)["patches"]
        }
        self.assertEqual(reviewed, tracked)

    def test_authority_digest_binds_linked_review_and_series(self) -> None:
        bundle = MODULE.load_bundle()
        self.assertNotEqual(
            MODULE.authority_sha256(bundle), MODULE.canonical_sha256(bundle)
        )


if __name__ == "__main__":
    unittest.main()
