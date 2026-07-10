#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate deterministic QuackGIS benchmark profile contracts without running them."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any


PROFILE_ID = re.compile(r"^layoutbench-[a-z0-9-]+-r([1-9][0-9]*)([kmb])-v[1-9][0-9]*$")
RFC3339 = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}[Tt][0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]+)?(?:[Zz]|[+-][0-9]{2}:[0-9]{2})$"
)
LEAP_SECOND_DATES = frozenset(
    {
        "1972-06-30", "1972-12-31", "1973-12-31", "1974-12-31",
        "1975-12-31", "1976-12-31", "1977-12-31", "1978-12-31",
        "1979-12-31", "1981-06-30", "1982-06-30", "1983-06-30",
        "1985-06-30", "1987-12-31", "1989-12-31", "1990-12-31",
        "1992-06-30", "1993-06-30", "1994-06-30", "1995-12-31",
        "1997-06-30", "1998-12-31", "2005-12-31", "2008-12-31",
        "2012-06-30", "2015-06-30", "2016-12-31",
    }
)
RUN_METADATA_FIELD = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
RESERVED_RUN_METADATA_FIELDS = {
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
ROW_MULTIPLIERS = {"k": 1_000, "m": 1_000_000, "b": 1_000_000_000}


def integer(value: Any, field: str, errors: list[str], *, positive: bool = False) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        errors.append(f"{field} must be an integer")
        return None
    if positive and value <= 0:
        errors.append(f"{field} must be positive")
        return None
    return value


def parse_rfc3339(value: str) -> datetime:
    if not RFC3339.fullmatch(value):
        raise ValueError("must use RFC3339 full-date/time syntax")
    normalized = f"{value[:10]}T{value[11:]}"
    if normalized[-1] in "Zz":
        normalized = f"{normalized[:-1]}+00:00"
    leap_second = normalized[17:19] == "60"
    if leap_second:
        normalized = f"{normalized[:17]}59{normalized[19:]}"
    parsed = datetime.fromisoformat(normalized)
    if leap_second:
        prior_utc = parsed.astimezone(timezone.utc)
        if (
            prior_utc.strftime("%Y-%m-%d") not in LEAP_SECOND_DATES
            or (prior_utc.hour, prior_utc.minute, prior_utc.second) != (23, 59, 59)
        ):
            raise ValueError("second 60 is not a recognized UTC leap-second boundary")
        parsed += timedelta(seconds=1)
    return parsed


def timestamp(value: Any, field: str, errors: list[str]) -> datetime | None:
    if not isinstance(value, str):
        errors.append(f"{field} must be an RFC3339 string")
        return None
    try:
        parsed = parse_rfc3339(value)
    except ValueError as error:
        errors.append(f"{field} is not RFC3339: {error}")
        return None
    if parsed.tzinfo is None:
        errors.append(f"{field} must include an explicit UTC offset")
        return None
    return parsed


def validate_profile(data: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if data.get("profile_version") != 1:
        errors.append("profile_version must be 1")
    if data.get("status") != "definition_only" or data.get("execution_required") is not True:
        errors.append("unexecuted profiles must remain status=definition_only with execution_required=true")

    profile_id = data.get("profile_id")
    match = PROFILE_ID.fullmatch(profile_id) if isinstance(profile_id, str) else None
    if not match:
        errors.append("profile_id must encode an exact row scale such as layoutbench-regional-r100m-v1")
    if isinstance(profile_id, str) and re.search(r"(^|[-_])sf1($|[-_])", profile_id, re.IGNORECASE):
        errors.append("profile_id must not use ambiguous sf1 naming")

    target_rows = integer(data.get("target_rows"), "target_rows", errors, positive=True)
    if match and target_rows is not None:
        encoded_rows = int(match.group(1)) * ROW_MULTIPLIERS[match.group(2)]
        if encoded_rows != target_rows:
            errors.append(f"profile_id encodes {encoded_rows} rows but target_rows is {target_rows}")

    tables = data.get("tables")
    table_rows = 0
    max_batch_rows = 0
    seen_tables: set[str] = set()
    if not isinstance(tables, list) or not tables:
        errors.append("tables must be a non-empty array")
    else:
        for index, table in enumerate(tables):
            field = f"tables[{index}]"
            if not isinstance(table, dict):
                errors.append(f"{field} must be an object")
                continue
            table_id = table.get("id")
            if not isinstance(table_id, str) or not table_id:
                errors.append(f"{field}.id must be a non-empty string")
            elif table_id in seen_tables:
                errors.append(f"{field}.id duplicates {table_id!r}")
            else:
                seen_tables.add(table_id)
            rows = integer(table.get("rows"), f"{field}.rows", errors, positive=True)
            batch_rows = integer(
                table.get("copy_batch_rows"), f"{field}.copy_batch_rows", errors, positive=True
            )
            batches = integer(
                table.get("expected_batches"), f"{field}.expected_batches", errors, positive=True
            )
            if rows is not None:
                table_rows += rows
            if batch_rows is not None:
                max_batch_rows = max(max_batch_rows, batch_rows)
            if rows is not None and batch_rows is not None and batches is not None:
                expected = math.ceil(rows / batch_rows)
                if batches != expected:
                    errors.append(f"{field}.expected_batches must be ceil(rows/copy_batch_rows)={expected}")
    if target_rows is not None and table_rows != target_rows:
        errors.append(f"table rows sum to {table_rows}, expected target_rows={target_rows}")

    generator = data.get("generator")
    if not isinstance(generator, dict):
        errors.append("generator must be an object")
    else:
        bounds = generator.get("bounds_m")
        if not isinstance(bounds, dict):
            errors.append("generator.bounds_m must be an object")
        else:
            values = {
                key: integer(bounds.get(key), f"generator.bounds_m.{key}", errors)
                for key in ("min_x", "min_y", "max_x", "max_y")
            }
            if all(value is not None for value in values.values()):
                if values["max_x"] <= values["min_x"] or values["max_y"] <= values["min_y"]:
                    errors.append("generator bounds must have positive width and height")
                if values["max_x"] - values["min_x"] > 1_000_000:
                    errors.append("generator regional X extent must not exceed 1,000 km")
                if values["max_y"] - values["min_y"] > 1_000_000:
                    errors.append("generator regional Y extent must not exceed 1,000 km")
        time_range = generator.get("time_range")
        if not isinstance(time_range, dict):
            errors.append("generator.time_range must be an object")
        else:
            start = timestamp(time_range.get("start"), "generator.time_range.start", errors)
            end = timestamp(time_range.get("end"), "generator.time_range.end", errors)
            if start is not None and end is not None and start >= end:
                errors.append("generator.time_range.start must be before end")

    storage = data.get("storage")
    row_group_rows = None
    if not isinstance(storage, dict):
        errors.append("storage must be an object")
    else:
        row_group_rows = integer(
            storage.get("row_group_rows"), "storage.row_group_rows", errors, positive=True
        )
        if storage.get("profile") != "postgresql-s3-compatible":
            errors.append("regional profile storage.profile must be postgresql-s3-compatible")
        if storage.get("compaction_during_load") is not False:
            errors.append("regional baseline must disable compaction during load")
    if row_group_rows is not None and max_batch_rows > row_group_rows:
        errors.append("copy batches must fit in one configured row group for the v1 file-count oracle")

    measurement = data.get("measurement")
    queries = None
    if not isinstance(measurement, dict):
        errors.append("measurement must be an object")
    else:
        queries = integer(
            measurement.get("warm_public_selective_queries"),
            "measurement.warm_public_selective_queries",
            errors,
            positive=True,
        )
        oracles = measurement.get("required_result_oracles")
        required_oracles = {"table_counts", "dataset_bounds", "exact_vs_pruned_counts"}
        if not isinstance(oracles, list) or not required_oracles.issubset(set(oracles)):
            errors.append("measurement.required_result_oracles is missing correctness oracles")

    budgets = data.get("budgets")
    required_budgets = {
        "warm_public_catalog_provider_calls_per_query_max",
        "warm_public_catalog_provider_calls_total_max",
        "measured_phase_catalog_refreshes_max",
        "cold_public_catalog_provider_calls_max",
        "direct_internal_catalog_provider_calls_max",
    }
    parsed_budgets: dict[str, int] = {}
    if not isinstance(budgets, dict):
        errors.append("budgets must be an object")
    else:
        missing = required_budgets - budgets.keys()
        if missing:
            errors.append(f"budgets missing {', '.join(sorted(missing))}")
        for key in required_budgets & budgets.keys():
            value = integer(budgets[key], f"budgets.{key}", errors)
            if value is not None:
                if value < 0:
                    errors.append(f"budgets.{key} must be non-negative")
                parsed_budgets[key] = value
        if parsed_budgets.get("measured_phase_catalog_refreshes_max") != 0:
            errors.append("measured_phase_catalog_refreshes_max must be zero")
    per_query = parsed_budgets.get("warm_public_catalog_provider_calls_per_query_max")
    total = parsed_budgets.get("warm_public_catalog_provider_calls_total_max")
    if queries is not None and per_query is not None and total != queries * per_query:
        errors.append(
            "warm catalog provider-call budget must equal query count times per-query budget"
        )

    required_run_metadata = data.get("required_run_metadata")
    required_fields = {
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
    }
    valid_metadata = (
        isinstance(required_run_metadata, list)
        and all(
            isinstance(field, str) and RUN_METADATA_FIELD.fullmatch(field)
            for field in required_run_metadata
        )
        and len(required_run_metadata) == len(set(required_run_metadata))
    )
    if not valid_metadata:
        errors.append("required_run_metadata must contain unique key-safe field names")
    elif RESERVED_RUN_METADATA_FIELDS.intersection(required_run_metadata):
        errors.append("required_run_metadata must not reuse measurement field names")
    elif not required_fields.issubset(set(required_run_metadata)):
        errors.append("required_run_metadata is missing release evidence fields")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profiles", nargs="+", type=Path)
    args = parser.parse_args(argv)

    failed = False
    for path in args.profiles:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            print(f"{path}: {error}", file=sys.stderr)
            failed = True
            continue
        if not isinstance(data, dict):
            print(f"{path}: profile must be a JSON object", file=sys.stderr)
            failed = True
            continue
        errors = validate_profile(data)
        if errors:
            failed = True
            for error in errors:
                print(f"{path}: {error}", file=sys.stderr)
        else:
            print(
                "benchmark_profile_ok "
                f"profile={data['profile_id']} rows={data['target_rows']} "
                f"tables={len(data['tables'])} batches={sum(t['expected_batches'] for t in data['tables'])}"
            )
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
