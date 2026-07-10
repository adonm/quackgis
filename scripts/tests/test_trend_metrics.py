#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "trend_metrics.py"
SPEC = importlib.util.spec_from_file_location("trend_metrics", MODULE_PATH)
assert SPEC and SPEC.loader
TREND = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TREND)


class TrendMetricsTests(unittest.TestCase):
    def test_profile_and_catalog_budgets_survive_rendering(self) -> None:
        artifact = {
            "run": {
                "report_kind": "benchmark",
                "github_run_id": 10,
                "github_run_attempt": 1,
                "github_sha": "abc123",
                "run_started_at": "2026-07-10T12:00:00Z",
            },
            "summary": {"passed": 1, "failed": 0, "not_run": 0},
            "checks": {
                "regional": {
                    "label": "Regional",
                    "status": "pass",
                    "metrics": {
                        "benchmark_profile": "layoutbench-regional-r100m-v1",
                        "dataset_rows": 100000000,
                        "catalog_read_provider_calls": 1697,
                        "catalog_read_provider_calls_budget": 1697,
                        "catalog_read_provider_calls_per_query_max": 7,
                        "catalog_read_provider_calls_per_query_max_budget": 7,
                        "cold_public_catalog_read_provider_calls": 13,
                        "cold_public_catalog_read_provider_calls_budget": 13,
                        "direct_internal_catalog_read_provider_calls": 4,
                        "direct_internal_catalog_read_provider_calls_budget": 4,
                        "catalog_provider_call_scope": "postgresql_metadata_provider_methods",
                        "catalog_refreshes": 0,
                        "catalog_refreshes_budget": 0,
                    },
                }
            },
        }
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "metrics.json"
            path.write_text(json.dumps(artifact), encoding="utf-8")
            rows = TREND.rows_for_metrics(path)
        self.assertEqual(rows[0]["benchmark_profile"], "layoutbench-regional-r100m-v1")
        dashboard = TREND.render_dashboard(rows)
        self.assertIn("layoutbench-regional-r100m-v1", dashboard)
        self.assertIn("catalog_read_provider_calls=1697/1697", dashboard)
        self.assertIn("catalog_read_provider_calls_per_query_max=7/7", dashboard)
        self.assertIn("cold_public_catalog_read_provider_calls=13/13", dashboard)
        self.assertIn("direct_internal_catalog_read_provider_calls=4/4", dashboard)
        self.assertIn("provider_scope=postgresql_metadata_provider_methods", dashboard)
        self.assertIn("catalog_refreshes=0/0", dashboard)
        self.assertIn("Catalog provider-call, roundtrip, and refresh budgets", dashboard)

    def test_latest_run_uses_numeric_ids_and_timestamp(self) -> None:
        rows = [
            {
                "check": "regional",
                "github_run_id": "9",
                "github_run_attempt": "1",
                "run_started_at": "2026-07-09T12:00:00Z",
            },
            {
                "check": "regional",
                "github_run_id": "10",
                "github_run_attempt": "1",
                "run_started_at": "2026-07-10T12:00:00Z",
            },
        ]
        latest = TREND.latest_by_check(rows)
        self.assertEqual(latest[0]["github_run_id"], "10")

    def test_osm_real_data_columns_survive_dashboard(self) -> None:
        artifact = {
            "run": {"report_kind": "compatibility", "github_sha": "abc123"},
            "summary": {"passed": 1, "failed": 0, "not_run": 0},
            "checks": {
                "osm_postgis_parity": {
                    "label": "OSM",
                    "status": "pass",
                    "metrics": {
                        "quackgis_points_named_count": 50,
                        "qgis_points_feature_count": 50,
                        "mvt_points_tile_bytes": 1000,
                        "mvt_points_attribute_ok": True,
                    },
                }
            },
        }
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "metrics.json"
            path.write_text(json.dumps(artifact), encoding="utf-8")
            rows = TREND.rows_for_metrics(path)
        self.assertEqual(rows[0]["quackgis_points_named_count"], 50)
        self.assertTrue(rows[0]["mvt_points_attribute_ok"])
        self.assertIn("Client real-data counts", TREND.render_dashboard(rows))


if __name__ == "__main__":
    unittest.main()
