#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "duckdb_engine_probe.py"
SPEC = importlib.util.spec_from_file_location("duckdb_engine_probe", MODULE_PATH)
assert SPEC and SPEC.loader
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


def fake_duckdb(temp: Path, *, fail: bool = False) -> Path:
    script = temp / "duckdb"
    if fail:
        body = "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'DuckDB v-test'; exit 0; fi\necho 'boom' >&2\nexit 1\n"
    else:
        body = (
            "#!/bin/sh\n"
            "if [ \"$1\" = \"--version\" ]; then echo 'DuckDB v-test'; exit 0; fi\n"
            "echo 'check_name,ok'\n"
            "echo 'ducklake_extension_loaded,1'\n"
            "echo 'engine_spatial_wkb,2,2,origin,one'\n"
        )
    script.write_text(body, encoding="utf-8")
    script.chmod(0o755)
    return script


class DuckDbEngineProbeTests(unittest.TestCase):
    def test_redacts_signed_url_and_database_secrets(self) -> None:
        text = "postgres://user:secret@example/db?X-Amz-Signature=abc password=plain"
        redacted = PROBE.redact(text)
        self.assertNotIn("secret", redacted)
        self.assertNotIn("abc", redacted)
        self.assertNotIn("plain", redacted)
        self.assertIn("postgres://user:<redacted>@example/db", redacted)
        self.assertIn("X-Amz-Signature=<redacted>", redacted)

    def test_fake_duckdb_success_writes_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            with contextlib.redirect_stdout(io.StringIO()):
                status = PROBE.main(["--duckdb-bin", str(fake_duckdb(temp)), "--out", str(out)])
            self.assertEqual(status, 0)
            report = out.read_text(encoding="utf-8")
            self.assertIn("Status: `pass`", report)
            self.assertIn("engine_spatial_wkb", report)

    def test_missing_duckdb_is_explicit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            with contextlib.redirect_stderr(io.StringIO()):
                status = PROBE.main(["--duckdb-bin", str(temp / "missing-duckdb"), "--out", str(out)])
            self.assertEqual(status, 2)
            self.assertIn("missing_duckdb", out.read_text(encoding="utf-8"))

    def test_duckdb_failure_writes_redacted_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            out = temp / "README.md"
            attach_sql = temp / "attach.sql"
            attach_sql.write_text("ATTACH 'postgres://user:secret@example/db';", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                status = PROBE.main(
                    [
                        "--duckdb-bin",
                        str(fake_duckdb(temp, fail=True)),
                        "--out",
                        str(out),
                        "--attach-sql",
                        str(attach_sql),
                    ]
                )
            self.assertEqual(status, 1)
            report = out.read_text(encoding="utf-8")
            self.assertIn("Status: `fail`", report)
            self.assertNotIn("secret", report)


if __name__ == "__main__":
    unittest.main()
