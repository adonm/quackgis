#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "build_native_bundle", ROOT / "scripts/build_native_bundle.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BuildNativeBundleTests(unittest.TestCase):
    def git(self, root: Path, *arguments: str) -> str:
        environment = {
            **os.environ,
            "GIT_AUTHOR_NAME": "Native Test",
            "GIT_AUTHOR_EMAIL": "native@example.invalid",
            "GIT_COMMITTER_NAME": "Native Test",
            "GIT_COMMITTER_EMAIL": "native@example.invalid",
        }
        return subprocess.run(
            ["git", *arguments],
            cwd=root,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def test_normalizes_only_the_exact_spatial_overlay(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            root = Path(directory)
            spatial = root / "spatial"
            (spatial / "vcpkg_ports").mkdir(parents=True)
            manifest = {
                "builtin-baseline": "a" * 40,
                "dependencies": ["roaring", "gdal"],
                "vcpkg-configuration": {
                    "overlay-ports": [str(spatial / "vcpkg_ports")]
                },
            }
            path = root / "vcpkg.json"
            path.write_text(json.dumps(manifest), encoding="utf-8")
            normalized = MODULE.normalized_vcpkg_manifest(path, spatial)
            self.assertEqual(
                normalized["vcpkg-configuration"]["overlay-ports"],
                ["${SPATIAL_SOURCE}/vcpkg_ports"],
            )

    def test_rejects_an_unowned_overlay(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            root = Path(directory)
            spatial = root / "spatial"
            spatial.mkdir()
            path = root / "vcpkg.json"
            path.write_text(
                json.dumps(
                    {
                        "vcpkg-configuration": {
                            "overlay-ports": [str(root / "other")]
                        }
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "unexpected overlay"):
                MODULE.normalized_vcpkg_manifest(path, spatial)

    def test_make_environment_uses_only_prepared_sources(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        sources = ROOT / ".tmp/native-bundle/sources"
        tool_paths = {
            "gcc": "/usr/bin/gcc",
            "g++": "/usr/bin/g++",
            "cmake": "/opt/cmake/bin/cmake",
            "ninja": "/opt/ninja/bin/ninja",
            "make": "/usr/bin/make",
        }
        with mock.patch.dict(
            os.environ,
            {
                "CC": "/malicious/cc",
                "CXX": "/malicious/c++",
                "CFLAGS": "-Dinjected",
                "VCPKG_BINARY_SOURCES": "clear;http,https://malicious.invalid,read",
            },
        ):
            environment = MODULE.make_environment(bundle, sources, tool_paths)
        variables = environment["EXTRA_CMAKE_VARIABLES"]
        self.assertIn(str((sources / "ducklake").resolve()), variables)
        self.assertIn(str((sources / "spatial").resolve()), variables)
        self.assertNotIn("GIT_URL", variables)
        self.assertEqual(environment["OVERRIDE_GIT_DESCRIBE"], "v1.5.4-0-g08e34c447b")
        self.assertNotEqual(environment["CC"], "/malicious/cc")
        self.assertNotEqual(environment["CXX"], "/malicious/c++")
        self.assertNotIn("CFLAGS", environment)
        self.assertEqual(environment["VCPKG_BINARY_SOURCES"], "clear")
        self.assertTrue(environment["VCPKG_DOWNLOADS"].startswith(str(ROOT / ".tmp")))
        self.assertEqual(environment["CC"], tool_paths["gcc"])
        self.assertTrue(environment["PATH"].startswith("/opt/cmake/bin:/opt/ninja/bin"))

    def test_rejects_ignored_source_side_input(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout = Path(directory)
            self.git(checkout, "init", "--quiet")
            (checkout / ".gitignore").write_text("*.cmake\n", encoding="utf-8")
            self.git(checkout, "add", ".gitignore")
            self.git(checkout, "commit", "--quiet", "-m", "base")
            (checkout / "injected.cmake").write_text("message(FATAL_ERROR)\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "ignored source-side inputs"):
                MODULE.reject_ignored_source_inputs(checkout, ())

    def test_rejects_staged_vcpkg_modification(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout = Path(directory)
            self.git(checkout, "init", "--quiet")
            (checkout / "tool.cmake").write_text("base\n", encoding="utf-8")
            self.git(checkout, "add", "tool.cmake")
            self.git(checkout, "commit", "--quiet", "-m", "base")
            commit = self.git(checkout, "rev-parse", "HEAD")
            self.git(checkout, "remote", "add", "origin", "https://example.invalid/vcpkg.git")
            (checkout / "tool.cmake").write_text("staged drift\n", encoding="utf-8")
            self.git(checkout, "add", "tool.cmake")
            with self.assertRaisesRegex(ValueError, "staged modifications"):
                MODULE.validate_vcpkg_checkout(
                    checkout,
                    {
                        "url": "https://example.invalid/vcpkg.git",
                        "commit": commit,
                    },
                )

    def test_rejects_hidden_vcpkg_modification(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            checkout = Path(directory)
            self.git(checkout, "init", "--quiet")
            (checkout / "tool.cmake").write_text("base\n", encoding="utf-8")
            self.git(checkout, "add", "tool.cmake")
            self.git(checkout, "commit", "--quiet", "-m", "base")
            commit = self.git(checkout, "rev-parse", "HEAD")
            self.git(checkout, "remote", "add", "origin", "https://example.invalid/vcpkg.git")
            self.git(checkout, "update-index", "--assume-unchanged", "tool.cmake")
            (checkout / "tool.cmake").write_text("hidden drift\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "non-default index flags"):
                MODULE.validate_vcpkg_checkout(
                    checkout,
                    {
                        "url": "https://example.invalid/vcpkg.git",
                        "commit": commit,
                    },
                )

    def test_cleanup_rejects_symlinked_parent(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            owner = Path(directory) / "owner"
            outside = Path(directory) / "outside"
            owner.mkdir()
            outside.mkdir()
            (owner / "build").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink|escapes"):
                MODULE.remove_owned_tree(owner / "build/release", owner, "test output")

    def test_vcpkg_checkout_rejects_symlinked_toolchain_parent(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
            prepared = Path(directory) / "bundle"
            outside = Path(directory) / "outside"
            prepared.mkdir()
            outside.mkdir()
            (prepared / "toolchain").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink|escapes"):
                MODULE.require_contained(
                    prepared / "toolchain/vcpkg", prepared, "vcpkg checkout"
                )

    def test_parses_actual_upstream_test_counts(self) -> None:
        result = MODULE.parse_test_result(
            "All tests passed (143 assertions in 3 test cases)\n", "ducklake"
        )
        self.assertEqual(result, {"assertions": 143, "test_cases": 3})
        with self.assertRaisesRegex(ValueError, "cannot parse"):
            MODULE.parse_test_result("No tests ran", "ducklake")
        with self.assertRaisesRegex(ValueError, "ran no assertions"):
            MODULE.parse_test_result(
                "All tests passed (0 assertions in 0 test cases)\n", "ducklake"
            )

    def test_every_declared_upstream_group_has_an_exact_filter(self) -> None:
        bundle = MODULE.native_bundle.load_bundle()
        filters = MODULE.candidate_test_filters(
            bundle, ROOT / ".tmp/native-bundle/sources"
        )
        self.assertTrue(set(bundle["test_groups"]["upstream"]).issubset(filters))
        self.assertEqual(
            str(filters["spatial-complete"]),
            str(ROOT / ".tmp/native-bundle/sources/spatial/test/sql/*"),
        )
        self.assertEqual(
            MODULE.candidate_test_requirement("ducklake-functions"), "ducklake"
        )
        self.assertEqual(
            MODULE.candidate_test_requirement("patch-ducklake-0-0"), "ducklake"
        )

    def test_extension_candidates_are_not_statically_linked(self) -> None:
        config = (ROOT / "native/extension_config.cmake").read_text(encoding="utf-8")
        self.assertEqual(config.count("DONT_LINK"), 2)


if __name__ == "__main__":
    unittest.main()
