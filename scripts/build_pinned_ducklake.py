#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Build and verify QuackGIS's source-pinned DuckLake identity extension."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

import native_bundle

ROOT = Path(__file__).resolve().parent.parent


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_authority(path: Path = native_bundle.BUNDLE_PATH) -> dict[str, object]:
    bundle = native_bundle.load_bundle(path.resolve(), ROOT)
    ducklake = bundle["extensions"]["ducklake"]
    series = native_bundle.validate_series(bundle, "ducklake", ROOT)
    if len(series["patches"]) != 1:
        raise ValueError("legacy DuckLake builder requires exactly one tracked patch")
    patch = series["patches"][0]
    provenance = ducklake["artifact"]["build_provenance"]
    if provenance["model"] != "legacy-separate":
        raise ValueError("legacy DuckLake builder cannot reproduce a central-build artifact")
    return {
        "schema_version": 1,
        "upstream_url": ducklake["source"]["url"],
        "upstream_commit": ducklake["source"]["commit"],
        "duckdb_commit": ducklake["duckdb_commit"],
        "patch": patch["path"],
        "patch_sha256": patch["sha256"],
        "artifact_sha256": ducklake["artifact"]["sha256"],
        "platform": bundle["platform"],
        "vcpkg_url": bundle["toolchain"]["vcpkg"]["url"],
        "vcpkg_commit": provenance["vcpkg_commit"],
        "cmake_version": provenance["cmake_version"],
        "ninja_version": provenance["ninja_version"],
    }


def run(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def require_checkout(path: Path, url: str, commit: str, label: str) -> None:
    if not (path / ".git").exists():
        raise ValueError(f"{label} checkout is missing at {path}")
    if output(["git", "rev-parse", "HEAD"], cwd=path) != commit:
        raise ValueError(f"{label} checkout at {path} is not pinned to {commit}")
    remotes = output(["git", "remote", "-v"], cwd=path)
    if url not in remotes:
        raise ValueError(f"{label} checkout at {path} does not name {url}")


def prepare_source(source: Path, vcpkg: Path, pin: dict[str, object]) -> None:
    marker = source / ".quackgis-pin.json"
    if source.exists():
        if not marker.is_file() or json.loads(marker.read_text(encoding="utf-8")) != pin:
            raise ValueError(
                f"refusing to reuse unrecognized source directory {source}; remove it explicitly"
            )
        require_checkout(
            source, str(pin["upstream_url"]), str(pin["upstream_commit"]), "DuckLake"
        )
        require_checkout(
            source / "duckdb",
            "https://github.com/duckdb/duckdb",
            str(pin["duckdb_commit"]),
            "DuckDB submodule",
        )
    else:
        source.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--no-checkout", str(pin["upstream_url"]), str(source)])
        run(["git", "checkout", "--detach", str(pin["upstream_commit"])], cwd=source)
        patch = ROOT / str(pin["patch"])
        run(["git", "apply", "--check", str(patch)], cwd=source)
        run(["git", "apply", str(patch)], cwd=source)
        run(
            ["git", "submodule", "update", "--init", "duckdb", "extension-ci-tools"],
            cwd=source,
        )
        run(["git", "checkout", "--detach", str(pin["duckdb_commit"])], cwd=source / "duckdb")
        marker.write_text(json.dumps(pin, indent=2) + "\n", encoding="utf-8")

    if vcpkg.exists():
        require_checkout(
            vcpkg, str(pin["vcpkg_url"]), str(pin["vcpkg_commit"]), "vcpkg"
        )
    else:
        vcpkg.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--no-checkout", str(pin["vcpkg_url"]), str(vcpkg)])
        run(["git", "checkout", "--detach", str(pin["vcpkg_commit"])], cwd=vcpkg)
        run([str(vcpkg / "bootstrap-vcpkg.sh"), "-disableMetrics"])


def build(source: Path, vcpkg: Path, pin: dict[str, object]) -> Path:
    run(
        [
            "make",
            "release",
            "GEN=ninja",
            f"VCPKG_TOOLCHAIN_PATH={vcpkg / 'scripts/buildsystems/vcpkg.cmake'}",
        ],
        cwd=source,
    )
    runner = source / "build/release/test/unittest"
    run([str(runner), "test/sql/functions/ducklake_column_info.test"], cwd=source)
    run([str(runner), "test/sql/functions/*"], cwd=source)
    artifact = source / "build/release/extension/ducklake/ducklake.duckdb_extension"
    if file_sha256(artifact) != pin["artifact_sha256"]:
        raise ValueError(
            "built DuckLake artifact does not match the accepted SHA-256; "
            "do not package it without reviewing and updating the artifact pin"
        )
    return artifact


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument(
        "--source", type=Path, default=ROOT / ".tmp/ref/quackgis-ducklake"
    )
    parser.add_argument("--vcpkg", type=Path, default=ROOT / ".tmp/ref/vcpkg-pinned")
    args = parser.parse_args(argv)
    try:
        pin = load_authority()
        if args.check:
            print(
                "pinned_ducklake_source_check_ok "
                f"base={pin['upstream_commit']} patch={pin['patch_sha256']} "
                f"artifact={pin['artifact_sha256']}"
            )
            return 0
        source = args.source.resolve()
        vcpkg = args.vcpkg.resolve()
        if ROOT not in source.parents or ROOT not in vcpkg.parents:
            raise ValueError("DuckLake and vcpkg build paths must remain workspace-local")
        prepare_source(source, vcpkg, pin)
        artifact = build(source, vcpkg, pin)
        print(
            f"pinned_ducklake_build_ok artifact={artifact} "
            f"sha256={pin['artifact_sha256']}"
        )
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"pinned DuckLake build failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
