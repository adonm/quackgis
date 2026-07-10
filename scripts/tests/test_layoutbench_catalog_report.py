#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import copy
import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "layoutbench_catalog_report.py"
SPEC = importlib.util.spec_from_file_location("layoutbench_catalog_report", MODULE_PATH)
assert SPEC and SPEC.loader
REPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPORT)
PROFILE = json.loads(
    (ROOT / "benchmarks" / "profiles" / "layoutbench-regional-r100m-v1.json").read_text(
        encoding="utf-8"
    )
)


def line(phase: str, queries: int, provider_calls: int, per_query_max: int) -> str:
    counter_start = 1000
    counter_end = counter_start + provider_calls
    return (
        f"layoutbench_catalog phase={phase} "
        "profile_id=layoutbench-regional-r100m-v1 target_rows=100000000 warm_queries=240 "
        "source_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "
        "storage_profile=postgresql-s3-compatible hardware_profile=kind-local-v1 "
        "memory_bytes=17179869184 free_disk_bytes=107374182400 object_bytes=123456789 "
        "elapsed_seconds=3600.5 github_run_id=123 github_run_attempt=1 "
        "run_started_at=2026-07-10T12:00:00Z correctness=pass "
        f"server_process_id=pod-{phase} "
        f"queries={queries} catalog_read_provider_calls={provider_calls} "
        f"catalog_read_provider_calls_per_query_max={per_query_max} catalog_refreshes=0 "
        f"catalog_read_provider_calls_start={counter_start} "
        f"catalog_read_provider_calls_end={counter_end}"
    )


def valid_log() -> str:
    return "\n".join(
        [
            line("cold_public", 1, 13, 13),
            line("direct_internal", 1, 4, 4),
            line("warm_public", 240, 1680, 7),
        ]
    )


class LayoutBenchCatalogReportTests(unittest.TestCase):
    def test_rfc3339_lowercase_and_leap_seconds_are_accepted(self) -> None:
        for value in (
            "2026-07-10t12:00:00z",
            "2016-12-31T23:59:60Z",
        ):
            with self.subTest(value=value):
                self.assertEqual(REPORT.require_timestamp(value, "run_started_at"), value)
        with self.assertRaises(REPORT.ContractError):
            REPORT.require_timestamp("2026-07-10T12:34:60Z", "run_started_at")

    def test_valid_report_preserves_read_only_suite_and_phase_budgets(self) -> None:
        artifact = REPORT.build_metrics(copy.deepcopy(PROFILE), valid_log())
        metrics = artifact["checks"]["layoutbench_catalog"]["metrics"]
        self.assertEqual(metrics["catalog_read_provider_calls"], 1697)
        self.assertEqual(metrics["catalog_read_provider_calls_budget"], 1697)
        self.assertEqual(metrics["catalog_read_provider_calls_per_query_max"], 7)
        self.assertEqual(metrics["cold_public_catalog_read_provider_calls"], 13)
        self.assertEqual(metrics["direct_internal_catalog_read_provider_calls"], 4)
        self.assertNotIn("catalog_roundtrips", metrics)
        self.assertNotIn("catalog_write_roundtrips", metrics)
        self.assertEqual(artifact["run"]["github_run_id"], 123)
        self.assertEqual(
            artifact["run"]["catalog_measurements"]["warm_public"]["counter_end"],
            2680,
        )

    def test_missing_duplicate_and_malformed_phases_fail_closed(self) -> None:
        cases = {
            "missing": "\n".join(valid_log().splitlines()[:2]),
            "duplicate": valid_log() + "\n" + line("warm_public", 240, 1680, 7),
            "malformed": valid_log().replace(" queries=1 ", " queries ", 1),
        }
        for label, log_text in cases.items():
            with self.subTest(label=label), self.assertRaises(REPORT.ContractError):
                REPORT.build_metrics(copy.deepcopy(PROFILE), log_text)

    def test_identity_correctness_and_metadata_are_required(self) -> None:
        cases = {
            "profile": valid_log().replace("layoutbench-regional-r100m-v1", "wrong", 1),
            "rows": valid_log().replace("target_rows=100000000", "target_rows=999", 1),
            "queries": valid_log().replace("warm_queries=240", "warm_queries=239", 1),
            "sha": valid_log().replace("a" * 40, "not-a-sha", 1),
            "correctness": valid_log().replace("correctness=pass", "correctness=fail", 1),
            "timestamp": valid_log().replace(
                "run_started_at=2026-07-10T12:00:00Z",
                "run_started_at=2026-W28-5T12:00:00Z",
                1,
            ),
        }
        for label, log_text in cases.items():
            with self.subTest(label=label), self.assertRaises(REPORT.ContractError):
                REPORT.build_metrics(copy.deepcopy(PROFILE), log_text)

    def test_every_catalog_budget_overrun_fails_closed(self) -> None:
        cases = {
            "cold": valid_log().replace(
                "catalog_read_provider_calls=13 catalog_read_provider_calls_per_query_max=13",
                "catalog_read_provider_calls=14 catalog_read_provider_calls_per_query_max=14",
            ),
            "direct": valid_log().replace(
                "catalog_read_provider_calls=4 catalog_read_provider_calls_per_query_max=4",
                "catalog_read_provider_calls=5 catalog_read_provider_calls_per_query_max=5",
            ),
            "warm_total": valid_log().replace(
                "catalog_read_provider_calls=1680", "catalog_read_provider_calls=1681"
            ),
            "warm_max": valid_log().replace(
                "catalog_read_provider_calls_per_query_max=7",
                "catalog_read_provider_calls_per_query_max=8",
            ),
            "refresh": valid_log().replace("catalog_refreshes=0", "catalog_refreshes=1", 1),
        }
        for label, log_text in cases.items():
            with self.subTest(label=label), self.assertRaises(REPORT.ContractError):
                REPORT.build_metrics(copy.deepcopy(PROFILE), log_text)

    def test_profile_and_raw_counter_contracts_fail_closed(self) -> None:
        invalid_profile = copy.deepcopy(PROFILE)
        invalid_profile["tables"][0]["rows"] -= 1
        with self.assertRaises(REPORT.ContractError):
            REPORT.build_metrics(invalid_profile, valid_log())

        wrong_identity = copy.deepcopy(PROFILE)
        wrong_identity["profile_id"] = "layoutbench-city-r100m-v1"
        with self.assertRaises(REPORT.ContractError):
            REPORT.build_metrics(wrong_identity, valid_log())

        extended_profile = copy.deepcopy(PROFILE)
        extended_profile["required_run_metadata"].append("image_digest")
        with self.assertRaises(REPORT.ContractError):
            REPORT.build_metrics(extended_profile, valid_log())
        extended_log = "\n".join(
            f"{record} image_digest=sha256:abc" for record in valid_log().splitlines()
        )
        artifact = REPORT.build_metrics(extended_profile, extended_log)
        self.assertEqual(
            artifact["run"]["required_metadata"]["image_digest"], "sha256:abc"
        )

        cases = {
            "reset": valid_log().replace(
                "catalog_read_provider_calls_end=1013",
                "catalog_read_provider_calls_end=999",
                1,
            ),
            "delta": valid_log().replace(
                "catalog_read_provider_calls_end=1013",
                "catalog_read_provider_calls_end=1012",
                1,
            ),
            "process": valid_log().replace("server_process_id=pod-cold_public", "server_process_id=bad$id"),
            "zero": valid_log().replace(
                "catalog_read_provider_calls=13",
                "catalog_read_provider_calls=0",
                1,
            ).replace(
                "catalog_read_provider_calls_end=1013",
                "catalog_read_provider_calls_end=1000",
                1,
            ),
        }
        for label, log_text in cases.items():
            with self.subTest(label=label), self.assertRaises(REPORT.ContractError):
                REPORT.build_metrics(copy.deepcopy(PROFILE), log_text)

    def test_cli_failure_removes_stale_output(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            profile = temp / "profile.json"
            log = temp / "catalog.log"
            output = temp / "metrics.json"
            profile.write_text(json.dumps(PROFILE), encoding="utf-8")
            log.write_text(line("cold_public", 1, 13, 13), encoding="utf-8")
            output.write_text("stale", encoding="utf-8")

            with contextlib.redirect_stderr(io.StringIO()):
                status = REPORT.main(
                    [
                        "--profile",
                        str(profile),
                        "--log",
                        str(log),
                        "--out",
                        str(output),
                    ]
                )

            self.assertEqual(status, 1)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
