#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import copy
import importlib.util
import json
import sys
import types
import unittest
from unittest import mock
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PROBES = ROOT / "deploy" / "kind" / "probes"
if str(PROBES) not in sys.path:
    sys.path.insert(0, str(PROBES))
if "psycopg2" not in sys.modules:
    fake_psycopg2 = types.ModuleType("psycopg2")
    fake_psycopg2.connect = lambda *args, **kwargs: (_ for _ in ()).throw(
        RuntimeError("psycopg2 is not available in static tests")
    )
    sys.modules["psycopg2"] = fake_psycopg2

PROBE_PATH = PROBES / "layoutbench_catalog_probe.py"
PROBE_SPEC = importlib.util.spec_from_file_location("layoutbench_catalog_probe", PROBE_PATH)
assert PROBE_SPEC and PROBE_SPEC.loader
PROBE = importlib.util.module_from_spec(PROBE_SPEC)
sys.modules["layoutbench_catalog_probe"] = PROBE
PROBE_SPEC.loader.exec_module(PROBE)

REPORT_PATH = ROOT / "scripts" / "layoutbench_catalog_report.py"
REPORT_SPEC = importlib.util.spec_from_file_location("layoutbench_catalog_report", REPORT_PATH)
assert REPORT_SPEC and REPORT_SPEC.loader
REPORT = importlib.util.module_from_spec(REPORT_SPEC)
REPORT_SPEC.loader.exec_module(REPORT)

PROFILE_JSON = json.loads(
    (ROOT / "benchmarks" / "profiles" / "layoutbench-regional-r100m-v1.json").read_text(
        encoding="utf-8"
    )
)


def profile() -> object:
    raw = copy.deepcopy(PROFILE_JSON)
    return PROBE.BenchmarkProfile(
        profile_id=raw["profile_id"],
        target_rows=raw["target_rows"],
        storage_profile=raw["storage"]["profile"],
        row_group_rows=raw["storage"]["row_group_rows"],
        warm_queries=raw["measurement"]["warm_public_selective_queries"],
        tables=tuple(
            PROBE.TableProfile(
                table_id=table["id"],
                rows=table["rows"],
                copy_batch_rows=table["copy_batch_rows"],
                expected_batches=table["expected_batches"],
            )
            for table in raw["tables"]
        ),
    )


class LayoutBenchCatalogKindProbeTests(unittest.TestCase):
    def test_profile_seed_plan_is_exact_and_bounded(self) -> None:
        loaded = profile()
        PROBE.validate_profile(loaded)
        self.assertEqual(sum(PROBE.copy_batches(table) for table in loaded.tables), 202)
        self.assertEqual(
            [PROBE.copy_batches(table) for table in loaded.tables],
            [95, 85, 22],
        )
        self.assertEqual(sum(rows for _, _, rows in PROBE.seed_plan(loaded)), 100_000_000)
        self.assertLessEqual(
            max(rows for _, _, rows in PROBE.seed_plan(loaded)),
            500_000,
        )

    def test_seed_refuses_without_exact_ack(self) -> None:
        loaded = profile()
        with mock.patch.dict("os.environ", {}, clear=True):
            with self.assertRaises(RuntimeError):
                PROBE.require_seed_ack(loaded)
        with mock.patch.dict(
            "os.environ",
            {"LAYOUTBENCH_ALLOW_EXACT_R100M": "true", "LAYOUTBENCH_MAX_ROWS": "999"},
            clear=True,
        ):
            with self.assertRaises(RuntimeError):
                PROBE.require_seed_ack(loaded)

    def test_prometheus_parser_fails_closed(self) -> None:
        body = "quackgis_catalog_read_provider_calls_total 17\n"
        self.assertEqual(PROBE.metric_int(body, "quackgis_catalog_read_provider_calls_total"), 17)
        with self.assertRaises(RuntimeError):
            PROBE.metric_int("", "quackgis_catalog_read_provider_calls_total")
        with self.assertRaises(RuntimeError):
            PROBE.metric_int(
                "quackgis_catalog_read_provider_calls_total 1\nquackgis_catalog_read_provider_calls_total{pod=\"other\"} 2\n",
                "quackgis_catalog_read_provider_calls_total",
            )

    def test_phase_line_matches_report_contract(self) -> None:
        loaded = profile()
        metadata = {
            "source_sha": "a" * 40,
            "storage_profile": loaded.storage_profile,
            "hardware_profile": "kind-local-v1",
            "memory_bytes": "17179869184",
            "free_disk_bytes": "107374182400",
            "object_bytes": "123456789",
            "elapsed_seconds": "3600.5",
            "github_run_id": "123",
            "github_run_attempt": "1",
            "run_started_at": "2026-07-10T12:00:00Z",
        }
        lines = [
            PROBE.phase_line(
                loaded,
                metadata,
                f"lake-{phase}",
                PROBE.PhaseResult(phase, queries, calls, max_calls, 0, 1000, 1000 + calls, 1),
            )
            for phase, queries, calls, max_calls in (
                ("cold_public", 1, 13, 13),
                ("direct_internal", 1, 4, 4),
                ("warm_public", 240, 1680, 7),
            )
        ]
        artifact = REPORT.build_metrics(copy.deepcopy(PROFILE_JSON), "\n".join(lines))
        metrics = artifact["checks"]["layoutbench_catalog"]["metrics"]
        self.assertEqual(metrics["catalog_read_provider_calls"], 1697)
        self.assertEqual(metrics["catalog_read_provider_calls_per_query_max"], 7)


if __name__ == "__main__":
    unittest.main()
