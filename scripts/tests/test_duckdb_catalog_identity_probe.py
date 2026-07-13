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
MODULE_PATH = ROOT / "scripts" / "duckdb_catalog_identity_probe.py"
SPEC = importlib.util.spec_from_file_location("duckdb_catalog_identity_probe", MODULE_PATH)
assert SPEC and SPEC.loader
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


def result(stdout: str, returncode: int = 0) -> PROBE.SqlResult:
    return PROBE.SqlResult(
        "pass" if returncode == 0 else "fail", stdout, "", returncode
    )


def successful_results() -> tuple[
    PROBE.SqlResult, list[PROBE.SqlResult], PROBE.SqlResult, PROBE.SqlResult
]:
    create = result(
        "check_name,schema_id,table_id,table_uuid\n"
        "identity_before,1,2,stable-table-uuid\n"
        "check_name,schema_id,table_id,table_uuid\n"
        "identity_after,1,2,stable-table-uuid\n"
        "check_name,data_file\n"
        "identity_file,/probe/before.parquet\n"
        "identity_file,/probe/after.parquet\n"
    )
    fields = [
        result("check_name,name,field_id\nfield_identity,old_column,2\n"),
        result("check_name,name,field_id\nfield_identity,new_column,2\n"),
    ]
    reopen = result(
        "check_name,schema_id,table_id,table_uuid\n"
        "identity_reopen,1,2,stable-table-uuid\n"
        "check_name,column_name,ordinal_position\n"
        "column_reopen,id,1\n"
        "column_reopen,new_column,2\n"
        "check_name,rows,values\n"
        'rows_reopen,2,"before,after"\n'
    )
    recreate = result(
        "check_name,schema_id,table_id,table_uuid\n"
        "identity_recreated,1,3,new-table-uuid\n"
    )
    return create, fields, reopen, recreate


def fake_duckdb(temp: Path, *, bad_reopen: bool = False) -> Path:
    script = temp / "duckdb"
    reopen_uuid = "wrong-table-uuid" if bad_reopen else "stable-table-uuid"
    script.write_text(
        f"""#!/bin/sh
if [ "$1" = "--version" ]; then echo 'DuckDB v1.5.4 fake'; exit 0; fi
case "$4" in
  *identity_before*)
    cat <<'EOF'
check_name,schema_id,table_id,table_uuid
identity_before,1,2,stable-table-uuid
check_name,schema_id,table_id,table_uuid
identity_after,1,2,stable-table-uuid
check_name,data_file
identity_file,/probe/before.parquet
identity_file,/probe/after.parquet
EOF
    ;;
  *before.parquet*)
    printf 'check_name,name,field_id\nfield_identity,old_column,2\n'
    ;;
  *after.parquet*)
    printf 'check_name,name,field_id\nfield_identity,new_column,2\n'
    ;;
  *identity_reopen*)
    cat <<'EOF'
check_name,schema_id,table_id,table_uuid
identity_reopen,1,2,{reopen_uuid}
check_name,column_name,ordinal_position
column_reopen,id,1
column_reopen,new_column,2
check_name,rows,values
rows_reopen,2,"before,after"
EOF
    ;;
  *identity_recreated*)
    printf 'check_name,schema_id,table_id,table_uuid\nidentity_recreated,1,3,new-table-uuid\n'
    ;;
  *) echo 'unexpected SQL' >&2; exit 1 ;;
esac
""",
        encoding="utf-8",
    )
    script.chmod(0o755)
    return script


class DuckDbCatalogIdentityProbeTests(unittest.TestCase):
    def test_validate_accepts_stable_rename_reopen_and_new_recreate(self) -> None:
        create, fields, reopen, recreate = successful_results()
        checks, evidence = PROBE.validate(create, fields, reopen, recreate)
        self.assertTrue(all(status == "pass" for status in checks.values()))
        self.assertEqual(evidence["column_field_ids"], {"new_column": [2], "old_column": [2]})

    def test_validate_rejects_changed_reopen_identity(self) -> None:
        create, fields, reopen, recreate = successful_results()
        reopen = result(reopen.stdout.replace("stable-table-uuid", "wrong-table-uuid"))
        checks, _ = PROBE.validate(create, fields, reopen, recreate)
        self.assertEqual(checks["independent_reopen_identity"], "fail")

    def test_fake_duckdb_success_writes_decision_manifest(self) -> None:
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
            self.assertEqual(
                packet["claim"], "ducklake_durable_catalog_identity_feasibility"
            )
            self.assertEqual(
                packet["decision"]["postgresql_oid"],
                "transactional compatibility registry required",
            )
            self.assertIn("Status: `pass`", out.read_text(encoding="utf-8"))

    def test_fake_duckdb_identity_drift_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            with contextlib.redirect_stderr(io.StringIO()):
                status = PROBE.main(
                    [
                        "--duckdb-bin",
                        str(fake_duckdb(temp, bad_reopen=True)),
                        "--workdir",
                        str(temp / "work"),
                        "--out",
                        str(temp / "README.md"),
                        "--manifest",
                        str(temp / "manifest.json"),
                    ]
                )
            self.assertEqual(status, 1)


if __name__ == "__main__":
    unittest.main()
