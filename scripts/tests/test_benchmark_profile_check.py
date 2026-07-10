#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "benchmark_profile_check.py"
SPEC = importlib.util.spec_from_file_location("benchmark_profile_check", MODULE_PATH)
assert SPEC and SPEC.loader
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)
PROFILE = json.loads(
    (ROOT / "benchmarks" / "profiles" / "layoutbench-regional-r100m-v1.json").read_text(
        encoding="utf-8"
    )
)


class BenchmarkProfileCheckTests(unittest.TestCase):
    def test_committed_profile_is_valid(self) -> None:
        self.assertEqual(CHECKER.validate_profile(PROFILE), [])

    def test_row_total_mismatch_fails(self) -> None:
        profile = copy.deepcopy(PROFILE)
        profile["tables"][0]["rows"] -= 1
        self.assertTrue(any("table rows sum" in error for error in CHECKER.validate_profile(profile)))

    def test_ambiguous_name_and_missing_budget_fail(self) -> None:
        profile = copy.deepcopy(PROFILE)
        profile["profile_id"] = "layoutbench-sf1-v1"
        del profile["budgets"]["cold_public_catalog_provider_calls_max"]
        errors = CHECKER.validate_profile(profile)
        self.assertTrue(any("ambiguous sf1" in error for error in errors))
        self.assertTrue(any("cold_public_catalog_provider_calls_max" in error for error in errors))

    def test_required_metadata_names_are_unique_key_safe_and_not_reserved(self) -> None:
        for field in ("phase", "x=y"):
            with self.subTest(field=field):
                profile = copy.deepcopy(PROFILE)
                profile["required_run_metadata"].append(field)
                self.assertTrue(CHECKER.validate_profile(profile))


if __name__ == "__main__":
    unittest.main()
