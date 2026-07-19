#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate QuackGIS's exact native bundle and ordered patch authority."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
BUNDLE_PATH = ROOT / "native/bundle.json"
HEX40 = re.compile(r"[0-9a-f]{40}")
HEX64 = re.compile(r"[0-9a-f]{64}")
COMPONENTS = ("duckdb", "ducklake", "spatial")
AUTHORITY_TOOLS = (
    "scripts/native_bundle.py",
    "scripts/prepare_native_bundle.py",
    "scripts/build_native_bundle.py",
    "scripts/package_native_bundle.py",
    "scripts/check_native_upstreams.py",
    "scripts/prepare_duckdb_runtime.py",
    "scripts/bootstrap_duckdb.py",
    "scripts/build_pinned_ducklake.py",
    "Justfile",
)


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


def publish_staged_directory(staged: Path, final: Path) -> None:
    if staged.is_symlink() or not staged.is_dir():
        raise ValueError("staged directory must be a non-symlink directory")
    if final.is_symlink() or (final.exists() and not final.is_dir()):
        raise ValueError("published directory must be absent or a non-symlink directory")
    if not final.exists():
        staged.rename(final)
        return
    libc = ctypes.CDLL(None, use_errno=True)
    renameat2 = getattr(libc, "renameat2", None)
    if renameat2 is None:
        raise ValueError("atomic directory exchange is unavailable on this platform")
    renameat2.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    renameat2.restype = ctypes.c_int
    result = renameat2(
        -100,
        os.fsencode(staged),
        -100,
        os.fsencode(final),
        2,
    )
    if result != 0:
        error = ctypes.get_errno()
        raise OSError(error, "atomic directory exchange failed")
    shutil.rmtree(staged)


def authority_sha256(bundle: dict[str, Any], root: Path = ROOT) -> str:
    authority = {
        "manifest": bundle,
        "patch_series": {
            component: validate_series(bundle, component, root)
            for component in COMPONENTS
        },
        "upstream_review": validate_upstream_review(bundle, root),
        "extension_config_sha256": file_sha256(
            root / bundle["build"]["extension_config"]
        ),
        "tool_sha256": {
            path: file_sha256(root / path) for path in AUTHORITY_TOOLS
        },
    }
    return canonical_sha256(authority)


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
            "upstream_review",
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
        {"version", "release_tag", "source", "patch_series", "artifact", "source_license"},
        "DuckDB bundle member",
    )
    if duckdb["release_tag"] != f"v{duckdb['version']}":
        raise ValueError("DuckDB release tag/version mismatch")
    if not isinstance(duckdb["source_license"], str) or not duckdb["source_license"].strip():
        raise ValueError("DuckDB source_license must be non-empty")
    duckdb_artifact = require_keys(
        duckdb["artifact"],
        {
            "mode",
            "archive_url",
            "archive_sha256",
            "library_sha256",
            "cli_archive_url",
            "cli_archive_sha256",
            "cli_sha256",
            "official_extension_sha256",
        },
        "DuckDB artifact",
    )
    if duckdb_artifact["mode"] != "vendor-built":
        raise ValueError("baseline DuckDB artifact mode must be vendor-built")
    require_hex(duckdb_artifact["archive_sha256"], HEX64, "DuckDB archive digest")
    require_hex(duckdb_artifact["library_sha256"], HEX64, "DuckDB library digest")
    require_hex(duckdb_artifact["cli_archive_sha256"], HEX64, "DuckDB CLI archive digest")
    require_hex(duckdb_artifact["cli_sha256"], HEX64, "DuckDB CLI digest")
    official_extensions = require_keys(
        duckdb_artifact["official_extension_sha256"],
        {"ducklake", "spatial"},
        "official bootstrap extension digests",
    )
    for name, digest in official_extensions.items():
        require_hex(digest, HEX64, f"official {name} extension digest")

    extensions = require_keys(bundle["extensions"], {"ducklake", "spatial"}, "extensions")
    core_commit = duckdb["source"]["commit"]
    for name in ("ducklake", "spatial"):
        expected = {"source", "duckdb_commit", "patch_series", "artifact", "source_license"}
        if name == "spatial":
            expected |= {"networking", "optional_modules", "bundled_dependencies"}
        extension = require_keys(extensions[name], expected, f"{name} bundle member")
        if not isinstance(extension["source_license"], str) or not extension["source_license"].strip():
            raise ValueError(f"{name} source_license must be non-empty")
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
        bundle["toolchain"],
        {
            "vcpkg",
            "compiler",
            "cmake_version",
            "ninja_version",
            "make_version",
            "executable_sha256",
            "acquisition",
        },
        "toolchain",
    )
    vcpkg = require_keys(toolchain["vcpkg"], {"url", "commit"}, "vcpkg")
    require_hex(vcpkg["commit"], HEX40, "vcpkg commit")
    executable_sha256 = require_keys(
        toolchain["executable_sha256"],
        {"gcc", "g++", "cmake", "ninja", "make"},
        "toolchain executable digests",
    )
    for name, digest in executable_sha256.items():
        require_hex(digest, HEX64, f"{name} executable digest")
    acquisition = require_keys(
        toolchain["acquisition"],
        set(executable_sha256),
        "toolchain acquisition",
    )
    if not all(isinstance(value, str) and value.strip() for value in acquisition.values()):
        raise ValueError("toolchain acquisition values must be non-empty strings")
    require_keys(toolchain["compiler"], {"family", "version"}, "compiler")
    build = require_keys(
        bundle["build"],
        {
            "type",
            "generator",
            "central_duckdb_checkout",
            "extension_config",
            "merged_vcpkg_sha256",
            "runtime_online_install",
        },
        "build",
    )
    if build["central_duckdb_checkout"] is not True or build["runtime_online_install"] is not False:
        raise ValueError("bundle must require one central checkout and deny runtime installs")
    config_path = root / safe_relative_path(build["extension_config"], "extension_config")
    if not config_path.is_file() or config_path.is_symlink():
        raise ValueError("central extension_config is missing or is a symlink")
    require_hex(build["merged_vcpkg_sha256"], HEX64, "merged vcpkg digest")

    tests = require_keys(bundle["test_groups"], {"upstream", "quackgis"}, "test_groups")
    for name, values in tests.items():
        if (
            not isinstance(values, list)
            or not values
            or len(values) != len(set(values))
            or not all(isinstance(value, str) and value.strip() for value in values)
        ):
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
    validate_upstream_review(bundle, root)
    return bundle


def validate_upstream_review(bundle: dict[str, Any], root: Path = ROOT) -> dict[str, Any]:
    relative = safe_relative_path(bundle["upstream_review"], "upstream_review")
    path = root / relative
    if not path.is_file() or path.is_symlink():
        raise ValueError("upstream review is missing or is a symlink")
    review = require_keys(
        json.loads(path.read_text(encoding="utf-8")),
        {
            "schema_version",
            "reviewed_at",
            "policy",
            "components",
            "capability_reviews",
            "patch_reviews",
            "blockers",
        },
        "upstream review",
    )
    if review["schema_version"] != 1 or not isinstance(review["reviewed_at"], str):
        raise ValueError("upstream review identity is invalid")
    if re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", review["reviewed_at"]) is None:
        raise ValueError("upstream review date must use YYYY-MM-DD")
    policy = require_keys(
        review["policy"],
        {
            "latest_supported_release_before_local_code",
            "unreleased_tips_are_evidence_only",
            "accepted_bundle_requires_review",
        },
        "upstream review policy",
    )
    if any(value is not True for value in policy.values()):
        raise ValueError("upstream-first review policy cannot be disabled")
    components = require_keys(
        review["components"], set(COMPONENTS), "upstream review components"
    )
    for name in COMPONENTS:
        component = require_keys(
            components[name],
            {
                "source_url",
                "selected_commit",
                "release_model",
                "selection",
                "latest_release",
                "observed_refs",
                "notes",
            },
            f"{name} upstream review",
        )
        source = source_for(bundle, name)
        if component["source_url"] != source["url"] or component["selected_commit"] != source["commit"]:
            raise ValueError(f"{name} upstream review does not match the selected source")
        refs = component["observed_refs"]
        if not isinstance(refs, dict) or not refs:
            raise ValueError(f"{name} upstream review must record observed refs")
        for ref, commit in refs.items():
            if not isinstance(ref, str) or not ref.startswith("refs/heads/"):
                raise ValueError(f"{name} upstream review contains an invalid ref")
            require_hex(commit, HEX40, f"{name} observed ref")
        latest = component["latest_release"]
        if name == "duckdb":
            latest = require_keys(latest, {"tag", "commit"}, "DuckDB latest release")
            if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", latest["tag"]):
                raise ValueError("DuckDB latest release tag is invalid")
            require_hex(latest["commit"], HEX40, "DuckDB latest release commit")
        elif latest is not None:
            raise ValueError(f"{name} must use its DuckDB-versioned release model")
        for field in ("release_model", "selection", "notes"):
            if not isinstance(component[field], str) or not component[field].strip():
                raise ValueError(f"{name} upstream review {field} must be non-empty")

    capabilities = review["capability_reviews"]
    if not isinstance(capabilities, list) or not capabilities:
        raise ValueError("upstream capability review must be non-empty")
    capability_ids: set[str] = set()
    for item in capabilities:
        capability = require_keys(
            item,
            {
                "id",
                "area",
                "local_implementation",
                "upstream_evidence",
                "disposition",
                "deletion_gate",
            },
            "upstream capability review",
        )
        if capability["id"] in capability_ids:
            raise ValueError("upstream capability review IDs must be unique")
        capability_ids.add(capability["id"])
        if capability["disposition"] not in {
            "adopt-upstream",
            "retain-upstream-gap",
            "reevaluate-and-delete",
        }:
            raise ValueError("upstream capability disposition is invalid")
        if any(not isinstance(capability[field], str) or not capability[field].strip() for field in capability):
            raise ValueError("upstream capability review fields must be non-empty")

    patch_reviews = review["patch_reviews"]
    if not isinstance(patch_reviews, list):
        raise ValueError("upstream patch reviews must be a list")
    reviewed_paths: set[str] = set()
    for item in patch_reviews:
        patch = require_keys(
            item,
            {
                "path",
                "upstream_status",
                "searched_commits",
                "disposition",
                "reason",
                "deletion_gate",
            },
            "upstream patch review",
        )
        safe_relative_path(patch["path"], "upstream patch review path")
        if patch["path"] in reviewed_paths or patch["disposition"] not in {"retain", "delete"}:
            raise ValueError("upstream patch review path/disposition is invalid")
        reviewed_paths.add(patch["path"])
        if not isinstance(patch["searched_commits"], list) or not patch["searched_commits"]:
            raise ValueError("upstream patch review must name searched commits")
        for commit in patch["searched_commits"]:
            require_hex(commit, HEX40, "upstream patch searched commit")
    tracked_paths = {
        patch["path"]
        for component in COMPONENTS
        for patch in validate_series(bundle, component, root)["patches"]
    }
    if reviewed_paths != tracked_paths:
        raise ValueError("every tracked native patch must have exactly one upstream review")
    if not isinstance(review["blockers"], list) or not all(
        isinstance(blocker, str) and blocker.strip() for blocker in review["blockers"]
    ):
        raise ValueError("upstream review blockers must be a string list")
    if bundle["status"] == "accepted" and review["blockers"]:
        raise ValueError("an accepted bundle cannot have unresolved upstream review blockers")
    return review


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
        f"sha256={canonical_sha256(bundle)} authority_sha256={authority_sha256(bundle)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
