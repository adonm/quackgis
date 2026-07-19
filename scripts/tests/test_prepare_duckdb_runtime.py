#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "prepare_duckdb_runtime", ROOT / "scripts/prepare_duckdb_runtime.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "new.rs"
        source.write_bytes(b"first")
        paths = b"new.rs\0"
        first = MODULE.source_state_sha256(b"status", b"diff", paths, root)
        source.write_bytes(b"second")
        second = MODULE.source_state_sha256(b"status", b"diff", paths, root)
        assert first != second
    with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
        root = Path(directory)
        outside = root / "outside"
        outside.mkdir()
        output = root / "output"
        output.symlink_to(outside, target_is_directory=True)
        try:
            MODULE.require_runtime_output(output)
        except ValueError as error:
            assert "symlink" in str(error)
        else:
            raise AssertionError("symlinked runtime output was accepted")
    with tempfile.TemporaryDirectory(dir=ROOT / ".tmp") as directory:
        root = Path(directory)
        final = root / "runtime"
        partial = root / ".runtime.partial"
        final.mkdir()
        (final / "marker").write_text("old", encoding="utf-8")
        partial.mkdir()
        (partial / "marker").write_text("new", encoding="utf-8")
        MODULE.publish_runtime_output(partial, final)
        assert (final / "marker").read_text(encoding="utf-8") == "new"
        assert not partial.exists()
    identity = MODULE.runtime_bundle_identity()
    assert identity["bundle_id"] == MODULE.BUNDLE["bundle_id"]
    assert identity["bundle_sha256"] == MODULE.native_bundle.canonical_sha256(MODULE.BUNDLE)
    assert identity["authority_sha256"] == MODULE.native_bundle.authority_sha256(MODULE.BUNDLE)
    assert identity["components"]["duckdb"]["commit"] == MODULE.BUNDLE["duckdb"]["source"]["commit"]
    assert identity["components"]["ducklake"]["patches"] == [
        {
            "path": MODULE.DUCKLAKE_PATCHES[0]["path"],
            "sha256": MODULE.DUCKLAKE_PATCHES[0]["sha256"],
        }
    ]
    assert (
        identity["selected_artifacts"]["ducklake"]["artifact"]["build_provenance"]
        == MODULE.DUCKLAKE["artifact"]["build_provenance"]
    )
    assert (
        identity["selected_artifacts"]["duckdb"]["artifact"]["cli_sha256"]
        == MODULE.BUNDLE["duckdb"]["artifact"]["cli_sha256"]
    )
    assert "toolchain" not in identity
    assert identity["unaccepted_candidate_configuration"]["toolchain"] == MODULE.BUNDLE["toolchain"]
    assert not any(str(ROOT) in str(value) for value in identity.values())
    print("prepare_duckdb_runtime_test_ok")


if __name__ == "__main__":
    main()
