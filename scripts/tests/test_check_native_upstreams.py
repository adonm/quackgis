#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "check_native_upstreams", ROOT / "scripts/check_native_upstreams.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CheckNativeUpstreamsTests(unittest.TestCase):
    def review(self):
        bundle = MODULE.native_bundle.load_bundle()
        return MODULE.native_bundle.validate_upstream_review(bundle)

    def recorded_state(self, review):
        return {
            name: dict(component["observed_refs"])
            for name, component in review["components"].items()
        }

    def test_recorded_state_passes(self) -> None:
        review = self.review()
        latest = review["components"]["duckdb"]["latest_release"]
        self.assertEqual(
            MODULE.compare_review(
                review,
                self.recorded_state(review),
                (latest["tag"], latest["commit"]),
            ),
            [],
        )

    def test_branch_drift_requires_new_review(self) -> None:
        review = self.review()
        state = self.recorded_state(review)
        state["spatial"]["refs/heads/v1.5-variegata"] = "f" * 40
        latest = review["components"]["duckdb"]["latest_release"]
        errors = MODULE.compare_review(
            review, state, (latest["tag"], latest["commit"])
        )
        self.assertTrue(any("spatial" in error and "moved" in error for error in errors))

    def test_new_release_requires_new_review(self) -> None:
        review = self.review()
        errors = MODULE.compare_review(
            review, self.recorded_state(review), ("v2.0.0", "a" * 40)
        )
        self.assertTrue(any("latest release moved" in error for error in errors))

    def test_latest_release_ignores_prerelease_tags(self) -> None:
        refs = {
            "refs/tags/v1.5.4": "a" * 40,
            "refs/tags/v2.0.0-dev1": "b" * 40,
            "refs/tags/v1.4.5": "c" * 40,
        }
        self.assertEqual(MODULE.latest_release_tag(refs), ("v1.5.4", "a" * 40))


if __name__ == "__main__":
    unittest.main()
