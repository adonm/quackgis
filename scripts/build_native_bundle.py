#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Configure or build the prepared native bundle from one DuckDB checkout."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import native_bundle
import prepare_native_bundle


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PREPARED = ROOT / ".tmp/native-bundle"
PLAN_NAME = "central-build-plan.json"


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def run(
    command: list[str], *, cwd: Path | None = None, environment: dict[str, str] | None = None
) -> None:
    subprocess.run(command, cwd=cwd, env=environment, check=True)


def tool_version(command: list[str], label: str) -> str:
    try:
        return output(command).splitlines()[0]
    except (OSError, subprocess.CalledProcessError, IndexError) as error:
        raise ValueError(f"required {label} tool is unavailable") from error


def verify_toolchain(bundle: dict[str, Any]) -> dict[str, str]:
    compiler = bundle["toolchain"]["compiler"]
    if compiler["family"] != "gcc":
        raise ValueError("only the manifest-pinned GCC toolchain is currently supported")
    actual_compiler = output(["c++", "-dumpfullversion", "-dumpversion"])
    if actual_compiler != compiler["version"]:
        raise ValueError(
            f"compiler drift: expected GCC {compiler['version']}, got {actual_compiler}"
        )
    cmake = tool_version(["cmake", "--version"], "CMake").removeprefix("cmake version ")
    ninja = tool_version(["ninja", "--version"], "Ninja")
    if cmake != bundle["toolchain"]["cmake_version"]:
        raise ValueError(f"CMake drift: expected {bundle['toolchain']['cmake_version']}, got {cmake}")
    if ninja != bundle["toolchain"]["ninja_version"]:
        raise ValueError(f"Ninja drift: expected {bundle['toolchain']['ninja_version']}, got {ninja}")
    return {"compiler": f"gcc {actual_compiler}", "cmake": cmake, "ninja": ninja}


def normalized_vcpkg_manifest(path: Path, spatial_source: Path) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    configuration = manifest.get("vcpkg-configuration")
    if not isinstance(configuration, dict):
        raise ValueError("merged vcpkg manifest is missing vcpkg-configuration")
    overlays = configuration.get("overlay-ports")
    if not isinstance(overlays, list):
        raise ValueError("merged vcpkg manifest is missing overlay ports")
    expected_overlay = (spatial_source / "vcpkg_ports").resolve()
    normalized = []
    for overlay in overlays:
        if not isinstance(overlay, str) or Path(overlay).resolve() != expected_overlay:
            raise ValueError(f"merged vcpkg manifest contains an unexpected overlay: {overlay!r}")
        normalized.append("${SPATIAL_SOURCE}/vcpkg_ports")
    configuration["overlay-ports"] = normalized
    return manifest


def make_environment(bundle: dict[str, Any], sources: Path) -> dict[str, str]:
    version = bundle["duckdb"]["version"]
    commit = bundle["duckdb"]["source"]["commit"][:10]
    return {
        **os.environ,
        "GEN": "ninja",
        "EXTENSION_CONFIGS": str((ROOT / bundle["build"]["extension_config"]).resolve()),
        "EXTRA_CMAKE_VARIABLES": " ".join(
            [
                f"-DQUACKGIS_DUCKLAKE_SOURCE={(sources / 'ducklake').resolve()}",
                f"-DQUACKGIS_SPATIAL_SOURCE={(sources / 'spatial').resolve()}",
                "-DENABLE_SANITIZER=FALSE",
                "-DENABLE_UBSAN=0",
            ]
        ),
        "OVERRIDE_GIT_DESCRIBE": f"v{version}-0-g{commit}",
    }


def configure(bundle: dict[str, Any], prepared: Path) -> dict[str, Any]:
    prepare_native_bundle.prepare(bundle, prepared, ROOT)
    tools = verify_toolchain(bundle)
    sources = prepared / "sources"
    core = sources / "duckdb"
    configuration = core / "build/extension_configuration"
    if configuration.exists():
        if configuration.is_symlink():
            raise ValueError("central extension configuration output cannot be a symlink")
        shutil.rmtree(configuration)
    run(["make", "extension_configuration"], cwd=core, environment=make_environment(bundle, sources))
    merged_path = configuration / "vcpkg.json"
    if not merged_path.is_file() or merged_path.is_symlink():
        raise ValueError("central DuckDB configuration did not emit merged vcpkg.json")
    merged = normalized_vcpkg_manifest(merged_path, sources / "spatial")
    merged_sha256 = native_bundle.canonical_sha256(merged)
    if merged_sha256 != bundle["build"]["merged_vcpkg_sha256"]:
        raise ValueError(
            "merged vcpkg graph drifted: "
            f"expected {bundle['build']['merged_vcpkg_sha256']}, got {merged_sha256}"
        )
    plan = {
        "schema_version": 1,
        "bundle_id": bundle["bundle_id"],
        "authority_sha256": native_bundle.authority_sha256(bundle, ROOT),
        "duckdb_commit": bundle["duckdb"]["source"]["commit"],
        "extensions": {
            name: {
                "commit": bundle["extensions"][name]["source"]["commit"],
                "result_tree": native_bundle.validate_series(bundle, name, ROOT)[
                    "result_tree"
                ],
            }
            for name in ("ducklake", "spatial")
        },
        "toolchain": tools,
        "vcpkg_commit": bundle["toolchain"]["vcpkg"]["commit"],
        "merged_vcpkg_sha256": merged_sha256,
        "merged_vcpkg": merged,
        "state": "configured",
    }
    (prepared / PLAN_NAME).write_text(
        json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return plan


def prepare_vcpkg(bundle: dict[str, Any], prepared: Path) -> Path:
    authority = bundle["toolchain"]["vcpkg"]
    target = prepared / "toolchain/vcpkg"
    if target.exists():
        if target.is_symlink() or not (target / ".git").is_dir():
            raise ValueError("refusing unrecognized vcpkg toolchain directory")
        if output(["git", "rev-parse", "HEAD"], cwd=target) != authority["commit"]:
            raise ValueError("prepared vcpkg checkout is not at the manifest commit")
        if output(["git", "remote", "get-url", "origin"], cwd=target) != authority["url"]:
            raise ValueError("prepared vcpkg origin drifted")
        if output(["git", "rev-parse", "--is-shallow-repository"], cwd=target) == "true":
            run(["git", "fetch", "--quiet", "--unshallow", "origin"], cwd=target)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--quiet", "--no-checkout", authority["url"], str(target)])
        run(["git", "checkout", "--quiet", "--detach", authority["commit"]], cwd=target)
    if output(["git", "rev-parse", "HEAD"], cwd=target) != authority["commit"]:
        raise ValueError("prepared vcpkg checkout changed during preparation")
    if output(["git", "rev-parse", "--is-shallow-repository"], cwd=target) != "false":
        raise ValueError("vcpkg needs complete pinned history for version-tree resolution")
    if subprocess.run(["git", "diff", "--quiet"], cwd=target, check=False).returncode != 0:
        raise ValueError("prepared vcpkg checkout contains tracked modifications")
    executable = target / "vcpkg"
    if not executable.is_file():
        run([str(target / "bootstrap-vcpkg.sh"), "-disableMetrics"], cwd=target)
    return target


def build(bundle: dict[str, Any], prepared: Path) -> dict[str, Any]:
    plan = configure(bundle, prepared)
    vcpkg = prepare_vcpkg(bundle, prepared)
    sources = prepared / "sources"
    environment = make_environment(bundle, sources)
    environment.update(
        {
            "USE_MERGED_VCPKG_MANIFEST": "1",
            "VCPKG_TOOLCHAIN_PATH": str(vcpkg / "scripts/buildsystems/vcpkg.cmake"),
        }
    )
    build_root = sources / "duckdb/build/release"
    if build_root.exists():
        if build_root.is_symlink():
            raise ValueError("central release build output cannot be a symlink")
        shutil.rmtree(build_root)
    run(["make", "release"], cwd=sources / "duckdb", environment=environment)
    candidates = {
        "duckdb": build_root / "duckdb",
        "libduckdb.so": build_root / "src/libduckdb.so",
        "ducklake.duckdb_extension": build_root / "extension/ducklake/ducklake.duckdb_extension",
        "spatial.duckdb_extension": build_root / "extension/spatial/spatial.duckdb_extension",
    }
    for name, path in candidates.items():
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"central build did not emit {name}: {path}")
    plan["state"] = "built-unaccepted"
    plan["candidate_artifacts"] = {
        name: native_bundle.file_sha256(path) for name, path in candidates.items()
    }
    (prepared / PLAN_NAME).write_text(
        json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return plan


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=native_bundle.BUNDLE_PATH)
    parser.add_argument("--prepared", type=Path, default=DEFAULT_PREPARED)
    parser.add_argument("--build", action="store_true", help="build the pinned graph after configuration")
    args = parser.parse_args(argv)
    try:
        bundle = native_bundle.load_bundle(args.manifest.resolve(), ROOT)
        prepared = prepare_native_bundle.require_workspace_output(args.prepared)
        plan = build(bundle, prepared) if args.build else configure(bundle, prepared)
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"native bundle build failed: {error}", file=sys.stderr)
        return 1
    print(
        "native_bundle_build_ok "
        f"bundle={bundle['bundle_id']} state={plan['state']} "
        f"merged_vcpkg_sha256={plan['merged_vcpkg_sha256']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
