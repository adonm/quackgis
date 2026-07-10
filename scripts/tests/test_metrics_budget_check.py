#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "metrics_budget_check.py"
SPEC = importlib.util.spec_from_file_location("metrics_budget_check", MODULE_PATH)
assert SPEC and SPEC.loader
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class MetricsBudgetCheckTests(unittest.TestCase):
    def test_catalog_roundtrip_and_refresh_overruns_fail(self) -> None:
        data = {
            "checks": {
                "regional": {
                    "status": "pass",
                    "metrics": {
                        "catalog_roundtrips": 8,
                        "catalog_roundtrips_budget": 7,
                        "catalog_refreshes": 1,
                        "catalog_refreshes_budget": 0,
                    },
                }
            }
        }
        errors, assertions, checks = CHECKER.check_metrics(
            Path("metrics.json"), data, allow_not_run=False
        )
        self.assertEqual(assertions, 2)
        self.assertEqual(checks, {"regional"})
        self.assertTrue(any("catalog_roundtrips=8" in error for error in errors))
        self.assertTrue(any("catalog_refreshes=1" in error for error in errors))

    def test_catalog_budget_without_measurement_fails(self) -> None:
        data = {
            "checks": {
                "regional": {
                    "status": "pass",
                    "metrics": {"catalog_roundtrips_budget": 7},
                }
            }
        }
        errors, assertions, _ = CHECKER.check_metrics(
            Path("metrics.json"), data, allow_not_run=False
        )
        self.assertEqual(assertions, 1)
        self.assertTrue(any("missing/non-numeric" in error for error in errors))

    def test_catalog_provider_call_budgets_are_enforced(self) -> None:
        data = {
            "checks": {
                "regional": {
                    "status": "pass",
                    "metrics": {
                        "catalog_read_provider_calls_per_query_max": 8,
                        "catalog_read_provider_calls_per_query_max_budget": 7,
                        "cold_public_catalog_read_provider_calls": 14,
                        "cold_public_catalog_read_provider_calls_budget": 13,
                        "direct_internal_catalog_read_provider_calls": 5,
                        "direct_internal_catalog_read_provider_calls_budget": 4,
                    },
                }
            }
        }
        errors, assertions, _ = CHECKER.check_metrics(
            Path("metrics.json"), data, allow_not_run=False
        )
        self.assertEqual(assertions, 3)
        self.assertEqual(len(errors), 3)

    def test_non_finite_metrics_and_budgets_fail_closed(self) -> None:
        for field, value in (
            ("catalog_read_provider_calls", "nan"),
            ("catalog_read_provider_calls_budget", "inf"),
        ):
            with self.subTest(field=field):
                metrics = {
                    "catalog_read_provider_calls": 1,
                    "catalog_read_provider_calls_budget": 1,
                    field: value,
                }
                data = {
                    "checks": {
                        "regional": {"status": "pass", "metrics": metrics}
                    }
                }
                errors, _, _ = CHECKER.check_metrics(
                    Path("metrics.json"), data, allow_not_run=False
                )
                self.assertTrue(errors)

    def test_large_integer_budget_comparison_keeps_exact_precision(self) -> None:
        data = {
            "checks": {
                "regional": {
                    "status": "pass",
                    "metrics": {
                        "catalog_read_provider_calls": " 9007199254740993 ",
                        "catalog_read_provider_calls_budget": " 9007199254740992 ",
                    },
                }
            }
        }
        errors, assertions, _ = CHECKER.check_metrics(
            Path("metrics.json"), data, allow_not_run=False
        )
        self.assertEqual(assertions, 1)
        self.assertEqual(len(errors), 1)


if __name__ == "__main__":
    unittest.main()
