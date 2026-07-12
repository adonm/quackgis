#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate the common QuackGIS roadmap evidence envelope."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


LEVELS = {"smoke", "local", "reference", "external"}
ENVIRONMENTS = {
    "host_process",
    "constrained_container",
    "kind",
    "managed_service",
}
OBJECT_SECTIONS = {"data", "correctness", "measurements", "budgets"}
SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")


def validate(manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    required = {
        "schema_version",
        "profile_id",
        "evidence_level",
        "execution_environment",
        "status",
        "source",
        "runtime",
        "host",
        *OBJECT_SECTIONS,
        "scope",
    }
    missing = sorted(required - manifest.keys())
    if missing:
        errors.append(f"missing required fields: {missing}")
        return errors
    if manifest["schema_version"] != 1:
        errors.append("schema_version must be 1")
    if not isinstance(manifest["profile_id"], str) or not manifest["profile_id"].strip():
        errors.append("profile_id must be a non-empty string")
    if manifest["evidence_level"] not in LEVELS:
        errors.append(f"invalid evidence_level: {manifest['evidence_level']!r}")
    if manifest["execution_environment"] not in ENVIRONMENTS:
        errors.append(
            f"invalid execution_environment: {manifest['execution_environment']!r}"
        )
    if manifest["status"] not in {"pass", "fail"}:
        errors.append("status must be pass or fail")
    for section in OBJECT_SECTIONS | {"source", "runtime", "host"}:
        if not isinstance(manifest[section], dict):
            errors.append(f"{section} must be an object")

    source = manifest["source"]
    if isinstance(source, dict):
        if not GIT_SHA.fullmatch(str(source.get("sha", ""))):
            errors.append("source.sha must be a full lowercase Git SHA")
        if not isinstance(source.get("dirty"), bool):
            errors.append("source.dirty must be boolean")
        elif source["dirty"]:
            for name in ("status_sha256", "diff_sha256"):
                if not SHA256.fullmatch(str(source.get(name, ""))):
                    errors.append(f"source.{name} must identify dirty state")
        if manifest["evidence_level"] in {"reference", "external"} and source.get(
            "dirty"
        ):
            errors.append("reference/external evidence requires a clean source tree")

    runtime = manifest["runtime"]
    if isinstance(runtime, dict):
        for name in ("duckdb_version", "platform"):
            if not isinstance(runtime.get(name), str) or not runtime[name]:
                errors.append(f"runtime.{name} must be a non-empty string")
        if not SHA256.fullmatch(str(runtime.get("libduckdb_sha256", ""))):
            errors.append("runtime.libduckdb_sha256 must be SHA-256")
        extensions = runtime.get("extensions")
        if not isinstance(extensions, dict):
            errors.append("runtime.extensions must be an object")
        else:
            for name in ("ducklake", "spatial"):
                if not SHA256.fullmatch(str(extensions.get(name, ""))):
                    errors.append(f"runtime.extensions.{name} must be SHA-256")
        if contains_key(runtime, "path"):
            errors.append("runtime evidence must not contain local path fields")

    host = manifest["host"]
    if isinstance(host, dict):
        if not isinstance(host.get("logical_cpus"), int) or host["logical_cpus"] <= 0:
            errors.append("host.logical_cpus must be positive")
        if manifest["evidence_level"] == "reference" and host.get(
            "storage"
        ) in {None, "", "unspecified"}:
            errors.append("reference evidence requires host.storage metadata")
    return errors


def contains_key(value: Any, target: str) -> bool:
    if isinstance(value, dict):
        return target in value or any(contains_key(child, target) for child in value.values())
    if isinstance(value, list):
        return any(contains_key(child, target) for child in value)
    return False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "manifest",
        type=Path,
        nargs="?",
        default=Path(".tmp/duckdb-current-benchmark/manifest.json"),
    )
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        if not isinstance(manifest, dict):
            raise ValueError("manifest root must be an object")
        errors = validate(manifest)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        errors = [str(error)]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(
        f"evidence_manifest_check_ok profile={manifest['profile_id']} "
        f"level={manifest['evidence_level']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
