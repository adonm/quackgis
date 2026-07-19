#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "prepare_native_bundle", ROOT / "scripts/prepare_native_bundle.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def git(*args: str, cwd: Path) -> str:
    environment = {
        **os.environ,
        "GIT_AUTHOR_NAME": "Bundle Test",
        "GIT_AUTHOR_EMAIL": "bundle@example.invalid",
        "GIT_COMMITTER_NAME": "Bundle Test",
        "GIT_COMMITTER_EMAIL": "bundle@example.invalid",
    }
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=environment,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


class PrepareNativeBundleTests(unittest.TestCase):
    def make_checkout(self, root: Path) -> tuple[Path, dict[str, str], dict[str, object]]:
        upstream = root / "upstream"
        upstream.mkdir()
        git("init", "--quiet", cwd=upstream)
        (upstream / "value.txt").write_text("base\n", encoding="utf-8")
        git("add", "value.txt", cwd=upstream)
        git("commit", "--quiet", "-m", "base", cwd=upstream)
        commit = git("rev-parse", "HEAD", cwd=upstream)
        tree = git("rev-parse", "HEAD^{tree}", cwd=upstream)
        checkout = root / "checkout"
        git("clone", "--quiet", str(upstream), str(checkout), cwd=root)
        source = {"url": str(upstream), "commit": commit, "tree": tree}
        series: dict[str, object] = {
            "base_tree": tree,
            "result_tree": tree,
            "patches": [],
        }
        return checkout, source, series

    def test_exact_clean_checkout_is_reusable(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout, source, series = self.make_checkout(Path(directory))
            result = MODULE.validate_checkout(checkout, source, series)
            self.assertEqual(result["result_tree"], source["tree"])

    def test_untracked_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout, source, series = self.make_checkout(Path(directory))
            (checkout / "extra.txt").write_text("drift\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "untracked files"):
                MODULE.validate_checkout(checkout, source, series)

    def test_staged_tree_drift_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout, source, series = self.make_checkout(Path(directory))
            (checkout / "value.txt").write_text("changed\n", encoding="utf-8")
            git("add", "value.txt", cwd=checkout)
            with self.assertRaisesRegex(ValueError, "result tree drifted"):
                MODULE.validate_checkout(checkout, source, series)

    def test_assume_unchanged_drift_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout, source, series = self.make_checkout(Path(directory))
            git("update-index", "--assume-unchanged", "value.txt", cwd=checkout)
            (checkout / "value.txt").write_text("hidden drift\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "non-default index flags"):
                MODULE.validate_checkout(checkout, source, series)

    def test_output_outside_workspace_tmp_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "workspace .tmp"):
            MODULE.require_workspace_output(Path(tempfile.gettempdir()) / "native-bundle")

    def test_symlinked_sources_parent_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            owner = Path(directory) / "bundle"
            outside = Path(directory) / "outside"
            owner.mkdir()
            outside.mkdir()
            (owner / "sources").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink|escapes"):
                MODULE.require_owned_path(owner / "sources", owner, "prepared sources")


if __name__ == "__main__":
    unittest.main()
