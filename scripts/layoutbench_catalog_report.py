#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate LayoutBench catalog provider-call logs and emit budgeted metrics."""

from __future__ import annotations

import argparse
import json
import math
import re
import shlex
import sys
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from benchmark_profile_check import parse_rfc3339, validate_profile  # noqa: E402


EXPECTED_PROFILE_ID = "layoutbench-regional-r100m-v1"
EXPECTED_ROWS = 100_000_000
EXPECTED_WARM_QUERIES = 240
PHASES = ("cold_public", "direct_internal", "warm_public")
RUN_METADATA_FIELDS = (
    "source_sha",
    "storage_profile",
    "hardware_profile",
    "memory_bytes",
    "free_disk_bytes",
    "object_bytes",
    "elapsed_seconds",
    "github_run_id",
    "github_run_attempt",
    "run_started_at",
)
BASE_LINE_FIELDS = {
    "phase",
    "profile_id",
    "target_rows",
    "warm_queries",
    "correctness",
    "queries",
    "catalog_read_provider_calls",
    "catalog_read_provider_calls_per_query_max",
    "catalog_refreshes",
    "server_process_id",
    "catalog_read_provider_calls_start",
    "catalog_read_provider_calls_end",
}


class ContractError(ValueError):
    """The profile or measurement log does not satisfy the report contract."""


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError(f"{label} must be an object")
    return value


def require_int(value: Any, label: str, *, positive: bool = False) -> int:
    if isinstance(value, bool):
        raise ContractError(f"{label} must be an integer")
    try:
        parsed = int(value)
    except (TypeError, ValueError) as error:
        raise ContractError(f"{label} must be an integer") from error
    if str(value).strip() != str(parsed):
        raise ContractError(f"{label} must be an integer")
    if parsed < 0 or (positive and parsed == 0):
        qualifier = "positive" if positive else "non-negative"
        raise ContractError(f"{label} must be {qualifier}")
    return parsed


def require_float(value: Any, label: str, *, positive: bool = False) -> float:
    if isinstance(value, bool):
        raise ContractError(f"{label} must be numeric")
    try:
        parsed = float(value)
    except (TypeError, ValueError) as error:
        raise ContractError(f"{label} must be numeric") from error
    if not math.isfinite(parsed):
        raise ContractError(f"{label} must be finite")
    if parsed < 0 or (positive and parsed == 0):
        qualifier = "positive" if positive else "non-negative"
        raise ContractError(f"{label} must be {qualifier}")
    return parsed


def require_timestamp(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise ContractError(f"{label} must be an RFC3339 string")
    try:
        parsed = parse_rfc3339(value)
    except ValueError as error:
        raise ContractError(f"{label} must be RFC3339: {error}") from error
    if parsed.tzinfo is None:
        raise ContractError(f"{label} must include an explicit UTC offset")
    return value


def validate_bound_profile(profile: dict[str, Any]) -> tuple[str, ...]:
    profile_errors = validate_profile(profile)
    if profile_errors:
        raise ContractError(f"profile contract failed: {'; '.join(profile_errors)}")
    if profile.get("profile_id") != EXPECTED_PROFILE_ID:
        raise ContractError(f"profile_id must be {EXPECTED_PROFILE_ID}")
    if require_int(profile.get("target_rows"), "target_rows") != EXPECTED_ROWS:
        raise ContractError(f"target_rows must be {EXPECTED_ROWS}")
    measurement = require_object(profile.get("measurement"), "measurement")
    if (
        require_int(
            measurement.get("warm_public_selective_queries"),
            "measurement.warm_public_selective_queries",
        )
        != EXPECTED_WARM_QUERIES
    ):
        raise ContractError(
            f"measurement.warm_public_selective_queries must be {EXPECTED_WARM_QUERIES}"
        )
    required_metadata = profile.get("required_run_metadata")
    if not isinstance(required_metadata, list) or not set(RUN_METADATA_FIELDS).issubset(
        required_metadata
    ):
        raise ContractError("profile required_run_metadata is incomplete")
    if BASE_LINE_FIELDS.intersection(required_metadata):
        raise ContractError("profile required_run_metadata reuses measurement fields")
    storage = require_object(profile.get("storage"), "storage")
    if not isinstance(storage.get("profile"), str) or not storage["profile"]:
        raise ContractError("storage.profile must be a non-empty string")
    budgets = require_object(profile.get("budgets"), "budgets")
    for key in (
        "warm_public_catalog_provider_calls_per_query_max",
        "warm_public_catalog_provider_calls_total_max",
        "measured_phase_catalog_refreshes_max",
        "cold_public_catalog_provider_calls_max",
        "direct_internal_catalog_provider_calls_max",
    ):
        require_int(budgets.get(key), f"budgets.{key}")
    return tuple(required_metadata)


def load_profile(path: Path) -> dict[str, Any]:
    try:
        profile = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"could not read profile {path}: {error}") from error
    profile = require_object(profile, "profile")
    validate_bound_profile(profile)
    return profile


def parse_phase_lines(
    text: str, required_metadata: tuple[str, ...]
) -> dict[str, dict[str, str]]:
    line_fields = BASE_LINE_FIELDS | set(required_metadata)
    phases: dict[str, dict[str, str]] = {}
    for line_number, raw_line in enumerate(text.splitlines(), start=1):
        stripped = raw_line.strip()
        if not stripped.startswith("layoutbench_catalog"):
            continue
        try:
            tokens = shlex.split(stripped)
        except ValueError as error:
            raise ContractError(f"line {line_number}: malformed quoting: {error}") from error
        if not tokens or tokens[0] != "layoutbench_catalog":
            raise ContractError(f"line {line_number}: malformed layoutbench_catalog record")
        values: dict[str, str] = {}
        for token in tokens[1:]:
            if "=" not in token:
                raise ContractError(f"line {line_number}: expected key=value token")
            key, value = token.split("=", 1)
            if not key or not value:
                raise ContractError(f"line {line_number}: empty key or value")
            if key in values:
                raise ContractError(f"line {line_number}: duplicate field {key}")
            values[key] = value
        missing = line_fields - values.keys()
        unknown = values.keys() - line_fields
        if missing:
            raise ContractError(
                f"line {line_number}: missing fields {', '.join(sorted(missing))}"
            )
        if unknown:
            raise ContractError(
                f"line {line_number}: unknown fields {', '.join(sorted(unknown))}"
            )
        phase = values["phase"]
        if phase not in PHASES:
            raise ContractError(f"line {line_number}: unknown phase {phase!r}")
        if phase in phases:
            raise ContractError(f"line {line_number}: duplicate phase {phase}")
        phases[phase] = values
    missing_phases = set(PHASES) - phases.keys()
    if missing_phases:
        raise ContractError(f"missing phases {', '.join(sorted(missing_phases))}")
    return phases


def build_metrics(profile: dict[str, Any], log_text: str) -> dict[str, Any]:
    required_metadata = validate_bound_profile(profile)
    phases = parse_phase_lines(log_text, required_metadata)
    budgets = require_object(profile["budgets"], "budgets")

    canonical_metadata = {field: phases[PHASES[0]][field] for field in required_metadata}
    for phase, values in phases.items():
        if values["profile_id"] != EXPECTED_PROFILE_ID:
            raise ContractError(f"{phase}: profile_id must be {EXPECTED_PROFILE_ID}")
        if require_int(values["target_rows"], f"{phase}.target_rows") != EXPECTED_ROWS:
            raise ContractError(f"{phase}: target_rows must be {EXPECTED_ROWS}")
        if (
            require_int(values["warm_queries"], f"{phase}.warm_queries")
            != EXPECTED_WARM_QUERIES
        ):
            raise ContractError(f"{phase}: warm_queries must be {EXPECTED_WARM_QUERIES}")
        if values["correctness"] != "pass":
            raise ContractError(f"{phase}: correctness must be pass")
        for field, expected in canonical_metadata.items():
            if values[field] != expected:
                raise ContractError(f"{phase}: run metadata field {field} is inconsistent")

    if not re.fullmatch(r"[0-9a-f]{40}", canonical_metadata["source_sha"]):
        raise ContractError("source_sha must be a 40-character lowercase hexadecimal Git SHA")
    if canonical_metadata["storage_profile"] != profile["storage"]["profile"]:
        raise ContractError("storage_profile does not match the benchmark profile")
    if not canonical_metadata["hardware_profile"].strip():
        raise ContractError("hardware_profile must not be empty")
    memory_bytes = require_int(canonical_metadata["memory_bytes"], "memory_bytes", positive=True)
    free_disk_bytes = require_int(
        canonical_metadata["free_disk_bytes"], "free_disk_bytes", positive=True
    )
    object_bytes = require_int(canonical_metadata["object_bytes"], "object_bytes", positive=True)
    elapsed_seconds = require_float(
        canonical_metadata["elapsed_seconds"], "elapsed_seconds", positive=True
    )
    github_run_id = require_int(
        canonical_metadata["github_run_id"], "github_run_id", positive=True
    )
    github_run_attempt = require_int(
        canonical_metadata["github_run_attempt"], "github_run_attempt", positive=True
    )
    run_started_at = require_timestamp(canonical_metadata["run_started_at"], "run_started_at")

    parsed: dict[str, dict[str, Any]] = {}
    for phase, values in phases.items():
        queries = require_int(values["queries"], f"{phase}.queries", positive=True)
        expected_queries = EXPECTED_WARM_QUERIES if phase == "warm_public" else 1
        if queries != expected_queries:
            raise ContractError(f"{phase}: queries must be {expected_queries}")
        provider_calls = require_int(
            values["catalog_read_provider_calls"],
            f"{phase}.catalog_read_provider_calls",
        )
        per_query_max = require_int(
            values["catalog_read_provider_calls_per_query_max"],
            f"{phase}.catalog_read_provider_calls_per_query_max",
        )
        refreshes = require_int(
            values["catalog_refreshes"], f"{phase}.catalog_refreshes"
        )
        counter_start = require_int(
            values["catalog_read_provider_calls_start"],
            f"{phase}.catalog_read_provider_calls_start",
        )
        counter_end = require_int(
            values["catalog_read_provider_calls_end"],
            f"{phase}.catalog_read_provider_calls_end",
        )
        process_id = values["server_process_id"]
        if not re.fullmatch(r"[A-Za-z0-9._:/-]{1,128}", process_id):
            raise ContractError(f"{phase}.server_process_id is invalid")
        if counter_end < counter_start:
            raise ContractError(f"{phase}: provider-call counter reset during measurement")
        if counter_end - counter_start != provider_calls:
            raise ContractError(f"{phase}: provider-call delta does not match start/end counters")
        if provider_calls < queries:
            raise ContractError(f"{phase}: provider-call total must be at least the query count")
        if provider_calls > queries * per_query_max:
            raise ContractError(f"{phase}: provider-call total exceeds queries times per-query max")
        if per_query_max > provider_calls:
            raise ContractError(f"{phase}: per-query max exceeds phase total")
        parsed[phase] = {
            "queries": queries,
            "provider_calls": provider_calls,
            "per_query_max": per_query_max,
            "refreshes": refreshes,
            "counter_start": counter_start,
            "counter_end": counter_end,
            "process_id": process_id,
        }

    limits = {
        "warm_total": require_int(
            budgets["warm_public_catalog_provider_calls_total_max"], "warm total budget"
        ),
        "warm_max": require_int(
            budgets["warm_public_catalog_provider_calls_per_query_max"],
            "warm per-query budget",
        ),
        "cold": require_int(
            budgets["cold_public_catalog_provider_calls_max"], "cold budget"
        ),
        "direct": require_int(
            budgets["direct_internal_catalog_provider_calls_max"], "direct budget"
        ),
        "refreshes": require_int(
            budgets["measured_phase_catalog_refreshes_max"], "refresh budget"
        ),
    }
    observed_limits = (
        ("warm_public provider calls", parsed["warm_public"]["provider_calls"], limits["warm_total"]),
        ("warm_public per-query max", parsed["warm_public"]["per_query_max"], limits["warm_max"]),
        ("cold_public provider calls", parsed["cold_public"]["provider_calls"], limits["cold"]),
        ("direct_internal provider calls", parsed["direct_internal"]["provider_calls"], limits["direct"]),
    )
    for label, observed, budget in observed_limits:
        if observed > budget:
            raise ContractError(f"{label} {observed} exceeds budget {budget}")
    refreshes = sum(phase["refreshes"] for phase in parsed.values())
    if refreshes > limits["refreshes"]:
        raise ContractError(f"catalog refreshes {refreshes} exceeds budget {limits['refreshes']}")

    suite_provider_calls = sum(phase["provider_calls"] for phase in parsed.values())
    suite_budget = limits["warm_total"] + limits["cold"] + limits["direct"]
    metrics = {
        "benchmark_profile": EXPECTED_PROFILE_ID,
        "dataset_rows": EXPECTED_ROWS,
        "storage_profile": canonical_metadata["storage_profile"],
        "hardware_profile": canonical_metadata["hardware_profile"],
        "memory_bytes": memory_bytes,
        "free_disk_bytes": free_disk_bytes,
        "object_bytes": object_bytes,
        "elapsed_seconds": elapsed_seconds,
        "warm_public_queries": EXPECTED_WARM_QUERIES,
        "catalog_provider_call_scope": "postgresql_metadata_provider_methods",
        "catalog_read_provider_calls": suite_provider_calls,
        "catalog_read_provider_calls_budget": suite_budget,
        "warm_public_catalog_read_provider_calls": parsed["warm_public"]["provider_calls"],
        "warm_public_catalog_read_provider_calls_budget": limits["warm_total"],
        "catalog_read_provider_calls_per_query_max": parsed["warm_public"]["per_query_max"],
        "catalog_read_provider_calls_per_query_max_budget": limits["warm_max"],
        "cold_public_catalog_read_provider_calls": parsed["cold_public"]["provider_calls"],
        "cold_public_catalog_read_provider_calls_budget": limits["cold"],
        "direct_internal_catalog_read_provider_calls": parsed["direct_internal"]["provider_calls"],
        "direct_internal_catalog_read_provider_calls_budget": limits["direct"],
        "catalog_refreshes": refreshes,
        "catalog_refreshes_budget": limits["refreshes"],
    }
    return {
        "run": {
            "report_kind": "benchmark",
            "github_run_id": github_run_id,
            "github_run_attempt": github_run_attempt,
            "github_sha": canonical_metadata["source_sha"],
            "run_started_at": run_started_at,
            "storage_recipe": canonical_metadata["storage_profile"],
            "required_metadata": canonical_metadata,
            "catalog_measurements": {
                phase: {
                    "server_process_id": values["process_id"],
                    "counter_start": values["counter_start"],
                    "counter_end": values["counter_end"],
                }
                for phase, values in parsed.items()
            },
        },
        "summary": {"passed": 1, "failed": 0, "not_run": 0},
        "checks": {
            "layoutbench_catalog": {
                "label": "LayoutBench regional catalog read provider calls",
                "status": "pass",
                "metrics": metrics,
            }
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--log", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        args.out.unlink(missing_ok=True)
        profile = load_profile(args.profile)
        log_text = args.log.read_text(encoding="utf-8")
        report = build_metrics(profile, log_text)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except (ContractError, OSError) as error:
        print(f"layoutbench catalog report failed: {error}", file=sys.stderr)
        return 1
    print(f"layoutbench_catalog_report_ok out={args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
