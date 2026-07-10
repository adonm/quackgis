#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
MODULE_PATH = ROOT / "scripts" / "duckdb_authority_probe.py"
SPEC = importlib.util.spec_from_file_location("duckdb_authority_probe", MODULE_PATH)
assert SPEC and SPEC.loader
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


def fake_duckdb(temp: Path, *, fail_reopen: bool = False) -> Path:
    script = temp / "duckdb"
    body = """#!/bin/sh
if [ "$1" = "--version" ]; then echo 'DuckDB v-authority'; exit 0; fi
case "$4" in
  *ducklake_snapshots*)
    if [ "FAIL_REOPEN" = "1" ]; then echo 'reopen failed' >&2; exit 1; fi
    echo 'check_name,rows,names,hits'
    echo 'reopen,2,"uno,two",2'
    echo 'snapshot_id,snapshot_time,schema_version,changes'
    echo '5,2026-07-10,2,{tables_deleted_from=[2]}'
    ;;
  *)
    echo 'check_name,rows'
    echo 'after_insert,3'
    echo 'check_name,rows,names'
    echo 'after_mutation,2,"uno,two"'
    echo 'table_name,schema_id,table_id,file_count,file_size_bytes,delete_file_count,delete_file_size_bytes'
    echo 'points,1,2,2,2281,1,1205'
    ;;
esac
exit 0
""".replace("FAIL_REOPEN", "1" if fail_reopen else "0")
    script.write_text(body, encoding="utf-8")
    script.chmod(0o755)
    return script


class DuckDbAuthorityProbeTests(unittest.TestCase):
    def test_validate_accepts_expected_duckdb_outputs(self) -> None:
        create = PROBE.SqlResult(
            "pass",
            'after_insert,3\nafter_mutation,2,"uno,two"\ntable_name\npoints\n',
            "",
            0,
        )
        reopen = PROBE.SqlResult(
            "pass",
            'reopen,2,"uno,two",2\nsnapshot_id\n{tables_deleted_from=[2]}\n',
            "",
            0,
        )
        self.assertTrue(all(status == "pass" for status in PROBE.validate(create, reopen).values()))

    def test_fake_duckdb_success_writes_manifest_and_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            manifest = temp / "manifest.json"
            with contextlib.redirect_stdout(io.StringIO()):
                status = PROBE.main(
                    [
                        "--duckdb-bin",
                        str(fake_duckdb(temp)),
                        "--workdir",
                        str(temp / "work"),
                        "--out",
                        str(out),
                        "--manifest",
                        str(manifest),
                    ]
                )
            self.assertEqual(status, 0)
            packet = json.loads(manifest.read_text(encoding="utf-8"))
            self.assertEqual(packet["claim"], "duckdb_storage_authority_vertical_slice")
            self.assertIn("Status: `pass`", out.read_text(encoding="utf-8"))

    def test_missing_duckdb_is_explicit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            manifest = temp / "manifest.json"
            with contextlib.redirect_stderr(io.StringIO()):
                status = PROBE.main(
                    [
                        "--duckdb-bin",
                        str(temp / "missing"),
                        "--workdir",
                        str(temp / "work"),
                        "--out",
                        str(out),
                        "--manifest",
                        str(manifest),
                    ]
                )
            self.assertEqual(status, 2)
            self.assertIn("duckdb_version", manifest.read_text(encoding="utf-8"))

    def test_reopen_failure_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            manifest = temp / "manifest.json"
            with contextlib.redirect_stderr(io.StringIO()):
                status = PROBE.main(
                    [
                        "--duckdb-bin",
                        str(fake_duckdb(temp, fail_reopen=True)),
                        "--workdir",
                        str(temp / "work"),
                        "--out",
                        str(out),
                        "--manifest",
                        str(manifest),
                    ]
                )
            self.assertEqual(status, 1)
            packet = json.loads(manifest.read_text(encoding="utf-8"))
            self.assertEqual(packet["checks"]["reopen"], "fail")


if __name__ == "__main__":
    unittest.main()
