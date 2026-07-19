#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Prepare exact native bundle sources for one central DuckDB build."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import native_bundle


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / ".tmp/native-bundle"
MARKER = "prepared-sources.json"


def run(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def require_workspace_output(path: Path) -> Path:
    resolved = path.resolve()
    temporary_root = (ROOT / ".tmp").resolve()
    if resolved == temporary_root or temporary_root not in resolved.parents:
        raise ValueError("native bundle output must remain below workspace .tmp")
    return resolved


def require_owned_path(path: Path, owner: Path, label: str) -> None:
    try:
        relative = path.relative_to(owner)
        path.resolve().relative_to(owner.resolve())
    except ValueError as error:
        raise ValueError(f"{label} escapes its preparation directory") from error
    current = owner
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"{label} traverses a symlink: {current}")


def component_owner(bundle: dict[str, Any], component: str) -> dict[str, Any]:
    return bundle["duckdb"] if component == "duckdb" else bundle["extensions"][component]


def validate_checkout(
    target: Path,
    source: dict[str, Any],
    series: dict[str, Any],
) -> dict[str, str]:
    if target.is_symlink() or not (target / ".git").is_dir():
        raise ValueError(f"prepared checkout is missing or unrecognized: {target}")
    if output(["git", "rev-parse", "HEAD"], cwd=target) != source["commit"]:
        raise ValueError(f"prepared checkout is not at {source['commit']}: {target}")
    if output(["git", "rev-parse", "HEAD^{tree}"], cwd=target) != source["tree"]:
        raise ValueError(f"prepared checkout base tree drifted: {target}")
    if output(["git", "remote", "get-url", "origin"], cwd=target) != source["url"]:
        raise ValueError(f"prepared checkout origin drifted: {target}")
    if output(["git", "ls-files", "--others", "--exclude-standard"], cwd=target):
        raise ValueError(f"prepared checkout contains untracked files: {target}")
    flagged = [
        line
        for line in output(["git", "ls-files", "-v"], cwd=target).splitlines()
        if not line.startswith("H ")
    ]
    if flagged:
        raise ValueError(
            f"prepared checkout contains non-default index flags: {target}: {flagged[:5]}"
        )
    refreshed = subprocess.run(
        ["git", "update-index", "--really-refresh"],
        cwd=target,
        check=False,
        capture_output=True,
    )
    if refreshed.returncode != 0:
        raise ValueError(f"prepared checkout tracked bytes differ from its index: {target}")
    unstaged = subprocess.run(
        ["git", "diff", "--quiet"], cwd=target, check=False
    ).returncode
    if unstaged != 0:
        raise ValueError(f"prepared checkout contains unstaged changes: {target}")
    run(["git", "diff", "--cached", "--check"], cwd=target)
    result_tree = output(["git", "write-tree"], cwd=target)
    if result_tree != series["result_tree"]:
        raise ValueError(
            f"prepared checkout result tree drifted: expected {series['result_tree']}, got {result_tree}"
        )
    if not series["patches"]:
        staged = subprocess.run(
            ["git", "diff", "--cached", "--quiet"], cwd=target, check=False
        ).returncode
        if staged != 0:
            raise ValueError(f"unpatched checkout contains staged changes: {target}")
    return {
        "commit": source["commit"],
        "base_tree": source["tree"],
        "result_tree": result_tree,
    }


def clone_and_patch(
    target: Path,
    source: dict[str, Any],
    series: dict[str, Any],
) -> dict[str, str]:
    partial = target.with_name(f".{target.name}.partial")
    if target.exists() or target.is_symlink():
        raise ValueError(f"refusing to replace existing source checkout: {target}")
    if partial.exists() or partial.is_symlink():
        raise ValueError(f"remove interrupted preparation explicitly: {partial}")
    partial.parent.mkdir(parents=True, exist_ok=True)
    try:
        run(["git", "init", "--quiet", str(partial)])
        run(["git", "remote", "add", "origin", source["url"]], cwd=partial)
        run(
            [
                "git",
                "fetch",
                "--quiet",
                "--depth",
                "1",
                "--filter=blob:none",
                "origin",
                source["commit"],
            ],
            cwd=partial,
        )
        run(
            ["git", "-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", "FETCH_HEAD"],
            cwd=partial,
        )
        if output(["git", "rev-parse", "HEAD"], cwd=partial) != source["commit"]:
            raise ValueError(f"source server did not resolve exact commit {source['commit']}")
        if output(["git", "rev-parse", "HEAD^{tree}"], cwd=partial) != source["tree"]:
            raise ValueError(f"source tree does not match manifest for {source['commit']}")
        for patch in series["patches"]:
            path = ROOT / patch["path"]
            run(["git", "apply", "--check", "--index", str(path)], cwd=partial)
            run(["git", "apply", "--index", str(path)], cwd=partial)
        result = validate_checkout(partial, source, series)
        partial.rename(target)
        return result
    except Exception:
        if partial.exists() and not partial.is_symlink():
            shutil.rmtree(partial)
        raise


def expected_marker(bundle: dict[str, Any], root: Path) -> dict[str, Any]:
    components: dict[str, Any] = {}
    for component in native_bundle.COMPONENTS:
        owner = component_owner(bundle, component)
        series = native_bundle.validate_series(bundle, component, root)
        components[component] = {
            "source_url": owner["source"]["url"],
            "commit": owner["source"]["commit"],
            "base_tree": owner["source"]["tree"],
            "result_tree": series["result_tree"],
            "patches": [
                {"path": patch["path"], "sha256": patch["sha256"]}
                for patch in series["patches"]
            ],
        }
    extension_config = root / bundle["build"]["extension_config"]
    return {
        "schema_version": 1,
        "bundle_id": bundle["bundle_id"],
        "bundle_sha256": native_bundle.canonical_sha256(bundle),
        "authority_sha256": native_bundle.authority_sha256(bundle, root),
        "extension_config_sha256": native_bundle.file_sha256(extension_config),
        "components": components,
    }


def prepare(bundle: dict[str, Any], out: Path, root: Path = ROOT) -> dict[str, Any]:
    out = require_workspace_output(out)
    marker = expected_marker(bundle, root)
    marker_path = out / MARKER
    sources = out / "sources"
    require_owned_path(sources, out, "prepared sources")
    if out.exists():
        if out.is_symlink() or not marker_path.is_file() or marker_path.is_symlink():
            raise ValueError(
                f"refusing to reuse unrecognized preparation directory {out}; remove it explicitly"
            )
        recorded = json.loads(marker_path.read_text(encoding="utf-8"))
        if recorded != marker:
            raise ValueError(
                f"prepared source authority changed for {out}; remove it explicitly and prepare again"
            )
        for component in native_bundle.COMPONENTS:
            owner = component_owner(bundle, component)
            series = native_bundle.validate_series(bundle, component, root)
            validate_checkout(sources / component, owner["source"], series)
        return marker

    sources.mkdir(parents=True)
    try:
        for component in native_bundle.COMPONENTS:
            owner = component_owner(bundle, component)
            series = native_bundle.validate_series(bundle, component, root)
            clone_and_patch(sources / component, owner["source"], series)
        marker_path.write_text(json.dumps(marker, indent=2) + "\n", encoding="utf-8")
    except Exception:
        if out.exists() and not out.is_symlink():
            shutil.rmtree(out)
        raise
    return marker


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=native_bundle.BUNDLE_PATH)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    try:
        bundle = native_bundle.load_bundle(args.manifest.resolve(), ROOT)
        marker = prepare(bundle, args.out, ROOT)
    except (
        OSError,
        ValueError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as error:
        print(f"native bundle preparation failed: {error}", file=sys.stderr)
        return 1
    print(
        "native_bundle_prepare_ok "
        f"bundle={marker['bundle_id']} sha256={marker['bundle_sha256']} out={args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
