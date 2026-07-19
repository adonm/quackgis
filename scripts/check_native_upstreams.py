#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare the recorded native adoption review with current upstream refs."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

import native_bundle


ROOT = Path(__file__).resolve().parent.parent
SEMVER_TAG = re.compile(r"refs/tags/(v([0-9]+)\.([0-9]+)\.([0-9]+))(\^\{\})?")


def git_ls_remote(url: str, refs: list[str]) -> dict[str, str]:
    result = subprocess.run(
        ["git", "ls-remote", url, *refs],
        check=True,
        capture_output=True,
        text=True,
    )
    found: dict[str, str] = {}
    for line in result.stdout.splitlines():
        commit, ref = line.split("\t", 1)
        found[ref] = commit
    return found


def latest_release_tag(refs: dict[str, str]) -> tuple[str, str]:
    versions: list[tuple[tuple[int, int, int], str, str]] = []
    peeled: dict[str, str] = {}
    direct: dict[str, str] = {}
    for ref, commit in refs.items():
        match = SEMVER_TAG.fullmatch(ref)
        if match is None:
            continue
        tag = match.group(1)
        version = tuple(int(match.group(index)) for index in (2, 3, 4))
        if match.group(5):
            peeled[tag] = commit
        else:
            direct[tag] = commit
        versions.append((version, tag, commit))
    if not versions:
        raise ValueError("DuckDB upstream exposes no stable semantic-version tags")
    _, tag, _ = max(versions, key=lambda item: item[0])
    return tag, peeled.get(tag, direct[tag])


def collect_remote_state(review: dict[str, Any]) -> tuple[dict[str, dict[str, str]], tuple[str, str]]:
    state: dict[str, dict[str, str]] = {}
    for name, component in review["components"].items():
        refs = list(component["observed_refs"])
        state[name] = git_ls_remote(component["source_url"], refs)
    duckdb_url = review["components"]["duckdb"]["source_url"]
    tags = git_ls_remote(duckdb_url, ["refs/tags/v*"])
    return state, latest_release_tag(tags)


def compare_review(
    review: dict[str, Any],
    remote_state: dict[str, dict[str, str]],
    latest_release: tuple[str, str],
) -> list[str]:
    errors: list[str] = []
    for name, component in review["components"].items():
        observed = component["observed_refs"]
        actual = remote_state.get(name, {})
        for ref, expected_commit in observed.items():
            actual_commit = actual.get(ref)
            if actual_commit != expected_commit:
                errors.append(
                    f"{name} {ref} moved: reviewed={expected_commit} current={actual_commit or 'missing'}"
                )
    reviewed_release = review["components"]["duckdb"]["latest_release"]
    if (reviewed_release["tag"], reviewed_release["commit"]) != latest_release:
        errors.append(
            "DuckDB latest release moved: "
            f"reviewed={reviewed_release['tag']}@{reviewed_release['commit']} "
            f"current={latest_release[0]}@{latest_release[1]}"
        )
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=native_bundle.BUNDLE_PATH)
    args = parser.parse_args(argv)
    try:
        bundle = native_bundle.load_bundle(args.manifest.resolve(), ROOT)
        review = native_bundle.validate_upstream_review(bundle, ROOT)
        state, latest = collect_remote_state(review)
        errors = compare_review(review, state, latest)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"native upstream check failed: {error}", file=sys.stderr)
        return 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(
            "upstream refs changed; inspect new released/compatible capabilities, "
            "update adoption/deletion decisions, then refresh native/upstream-review.json",
            file=sys.stderr,
        )
        return 1
    print(
        "native_upstream_check_ok "
        f"reviewed_at={review['reviewed_at']} latest_duckdb={latest[0]} "
        f"capabilities={len(review['capability_reviews'])} patches={len(review['patch_reviews'])}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
