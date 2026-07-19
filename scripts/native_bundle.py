#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate QuackGIS's exact native bundle and ordered patch authority."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
BUNDLE_PATH = ROOT / "native/bundle.json"
HEX40 = re.compile(r"[0-9a-f]{40}")
HEX64 = re.compile(r"[0-9a-f]{64}")
COMPONENTS = ("duckdb", "ducklake", "spatial")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha256(value: Any) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("ascii")
    return hashlib.sha256(encoded).hexdigest()


def require_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        actual = sorted(value) if isinstance(value, dict) else type(value).__name__
        raise ValueError(f"{label} has unsupported fields: {actual}")
    return value


def require_hex(value: Any, pattern: re.Pattern[str], label: str) -> str:
    if not isinstance(value, str) or pattern.fullmatch(value) is None:
        raise ValueError(f"{label} must be lowercase hexadecimal")
    return value


def safe_relative_path(value: Any, label: str) -> Path:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} must be a non-empty relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or str(path) != value:
        raise ValueError(f"{label} must be a normalized relative path")
    return path


def source_for(bundle: dict[str, Any], component: str) -> dict[str, Any]:
    source = bundle["duckdb"]["source"] if component == "duckdb" else bundle["extensions"][component]["source"]
    return require_keys(source, {"url", "commit", "tree"}, f"{component} source")


def validate_series(
    bundle: dict[str, Any], component: str, root: Path
) -> dict[str, Any]:
    owner = bundle["duckdb"] if component == "duckdb" else bundle["extensions"][component]
    relative = safe_relative_path(owner["patch_series"], f"{component} patch_series")
    path = root / relative
    if not path.is_file() or path.is_symlink():
        raise ValueError(f"{component} patch series is missing or is a symlink")
    series = require_keys(
        json.loads(path.read_text(encoding="utf-8")),
        {"schema_version", "component", "base_commit", "base_tree", "result_tree", "patches"},
        f"{component} patch series",
    )
    if series["schema_version"] != 1 or series["component"] != component:
        raise ValueError(f"{component} patch series identity is invalid")
    source = source_for(bundle, component)
    for field in ("commit", "tree"):
        require_hex(source[field], HEX40, f"{component} source {field}")
    if series["base_commit"] != source["commit"] or series["base_tree"] != source["tree"]:
        raise ValueError(f"{component} patch series does not match its source base")
    require_hex(series["result_tree"], HEX40, f"{component} result_tree")
    patches = series["patches"]
    if not isinstance(patches, list):
        raise ValueError(f"{component} patches must be a list")
    seen: set[str] = set()
    for index, item in enumerate(patches):
        patch = require_keys(
            item,
            {"path", "sha256", "requirement", "owner", "tests", "upstream_or_deletion"},
            f"{component} patch {index}",
        )
        patch_path = safe_relative_path(patch["path"], f"{component} patch path")
        if patch_path.suffix != ".patch" or str(patch_path) in seen:
            raise ValueError(f"{component} patch paths must be unique .patch files")
        seen.add(str(patch_path))
        absolute = root / patch_path
        if not absolute.is_file() or absolute.is_symlink():
            raise ValueError(f"{component} patch is missing or is a symlink: {patch_path}")
        expected = require_hex(patch["sha256"], HEX64, f"{component} patch sha256")
        if file_sha256(absolute) != expected:
            raise ValueError(f"{component} patch checksum drifted: {patch_path}")
        for field in ("requirement", "owner", "upstream_or_deletion"):
            if not isinstance(patch[field], str) or not patch[field].strip():
                raise ValueError(f"{component} patch {field} must be non-empty")
        if not isinstance(patch["tests"], list) or not patch["tests"] or not all(
            isinstance(test, str) and test.strip() for test in patch["tests"]
        ):
            raise ValueError(f"{component} patch tests must be a non-empty string list")
    if not patches and series["result_tree"] != series["base_tree"]:
        raise ValueError(f"unpatched {component} must retain its base tree")
    return series


def load_bundle(path: Path = BUNDLE_PATH, root: Path = ROOT) -> dict[str, Any]:
    bundle = require_keys(
        json.loads(path.read_text(encoding="utf-8")),
        {
            "schema_version",
            "bundle_id",
            "status",
            "platform",
            "duckdb",
            "extensions",
            "quackgis_extension",
            "toolchain",
            "build",
            "test_groups",
            "outputs",
        },
        "native bundle",
    )
    if bundle["schema_version"] != 1:
        raise ValueError("native bundle schema_version must be 1")
    if not isinstance(bundle["bundle_id"], str) or not re.fullmatch(
        r"[a-z0-9][a-z0-9.-]{7,127}", bundle["bundle_id"]
    ):
        raise ValueError("native bundle_id is invalid")
    if bundle["status"] not in {"candidate", "accepted"}:
        raise ValueError("native bundle status must be candidate or accepted")
    if bundle["platform"] != "linux-amd64":
        raise ValueError("native bundle currently supports only linux-amd64")

    duckdb = require_keys(
        bundle["duckdb"],
        {"version", "release_tag", "source", "patch_series", "artifact", "license"},
        "DuckDB bundle member",
    )
    if duckdb["release_tag"] != f"v{duckdb['version']}":
        raise ValueError("DuckDB release tag/version mismatch")
    duckdb_artifact = require_keys(
        duckdb["artifact"],
        {"mode", "archive_url", "archive_sha256", "library_sha256"},
        "DuckDB artifact",
    )
    if duckdb_artifact["mode"] != "vendor-built":
        raise ValueError("baseline DuckDB artifact mode must be vendor-built")
    require_hex(duckdb_artifact["archive_sha256"], HEX64, "DuckDB archive digest")
    require_hex(duckdb_artifact["library_sha256"], HEX64, "DuckDB library digest")

    extensions = require_keys(bundle["extensions"], {"ducklake", "spatial"}, "extensions")
    core_commit = duckdb["source"]["commit"]
    for name in ("ducklake", "spatial"):
        expected = {"source", "duckdb_commit", "patch_series", "artifact", "license"}
        if name == "spatial":
            expected |= {"networking", "optional_modules", "bundled_dependencies"}
        extension = require_keys(extensions[name], expected, f"{name} bundle member")
        if extension["duckdb_commit"] != core_commit:
            raise ValueError(f"{name} targets a different DuckDB commit")
        artifact_fields = {"mode", "signed", "sha256"}
        if name == "ducklake":
            artifact_fields.add("build_provenance")
        artifact = require_keys(extension["artifact"], artifact_fields, f"{name} artifact")
        require_hex(artifact["sha256"], HEX64, f"{name} artifact digest")
        if artifact["mode"] not in {"vendor-built", "project-built"}:
            raise ValueError(f"{name} artifact mode is invalid")
        if not isinstance(artifact["signed"], bool):
            raise ValueError(f"{name} signed flag must be boolean")
        if artifact["mode"] == "project-built" and artifact["signed"]:
            raise ValueError(f"project-built {name} cannot retain a vendor signature claim")
        if name == "ducklake":
            provenance = require_keys(
                artifact["build_provenance"],
                {"model", "vcpkg_commit", "cmake_version", "ninja_version", "compiler"},
                "ducklake artifact build_provenance",
            )
            if provenance["model"] not in {"legacy-separate", "central"}:
                raise ValueError("DuckLake artifact build model is invalid")
            require_hex(provenance["vcpkg_commit"], HEX40, "DuckLake artifact vcpkg commit")
            if bundle["status"] == "accepted" and provenance["model"] != "central":
                raise ValueError("an accepted bundle cannot use legacy separate-build provenance")

    quackgis = require_keys(
        bundle["quackgis_extension"], {"enabled", "reason"}, "QuackGIS extension"
    )
    if quackgis["enabled"] is not False or not isinstance(quackgis["reason"], str):
        raise ValueError("this baseline must explicitly explain its disabled QuackGIS extension")

    toolchain = require_keys(
        bundle["toolchain"], {"vcpkg", "compiler", "cmake_version", "ninja_version"}, "toolchain"
    )
    vcpkg = require_keys(toolchain["vcpkg"], {"url", "commit"}, "vcpkg")
    require_hex(vcpkg["commit"], HEX40, "vcpkg commit")
    require_keys(toolchain["compiler"], {"family", "version"}, "compiler")
    build = require_keys(
        bundle["build"],
        {"type", "generator", "central_duckdb_checkout", "extension_config", "runtime_online_install"},
        "build",
    )
    if build["central_duckdb_checkout"] is not True or build["runtime_online_install"] is not False:
        raise ValueError("bundle must require one central checkout and deny runtime installs")
    config_path = root / safe_relative_path(build["extension_config"], "extension_config")
    if not config_path.is_file() or config_path.is_symlink():
        raise ValueError("central extension_config is missing or is a symlink")

    tests = require_keys(bundle["test_groups"], {"upstream", "quackgis"}, "test_groups")
    for name, values in tests.items():
        if not isinstance(values, list) or not values or len(values) != len(set(values)):
            raise ValueError(f"{name} test group must be a non-empty unique list")
    outputs = require_keys(
        bundle["outputs"],
        {"runtime_manifest", "sbom", "license_inventory", "spdx_created"},
        "outputs",
    )
    for name in ("runtime_manifest", "sbom", "license_inventory"):
        value = outputs[name]
        safe_relative_path(value, f"output {name}")
    if not isinstance(outputs["spdx_created"], str) or re.fullmatch(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",
        outputs["spdx_created"],
    ) is None:
        raise ValueError("output spdx_created must be a stable UTC timestamp")

    for component in COMPONENTS:
        source = source_for(bundle, component)
        if not isinstance(source["url"], str) or not source["url"].startswith("https://github.com/"):
            raise ValueError(f"{component} source URL must be an HTTPS GitHub URL")
        validate_series(bundle, component, root)
    return bundle


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=BUNDLE_PATH)
    args = parser.parse_args(argv)
    try:
        bundle = load_bundle(args.manifest.resolve(), ROOT)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"native bundle check failed: {error}", file=sys.stderr)
        return 1
    print(
        "native_bundle_check_ok "
        f"bundle={bundle['bundle_id']} status={bundle['status']} "
        f"sha256={canonical_sha256(bundle)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
