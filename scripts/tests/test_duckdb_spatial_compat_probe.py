#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "duckdb_spatial_compat_probe", ROOT / "scripts/duckdb_spatial_compat_probe.py"
)
assert SPEC and SPEC.loader
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class DuckDbSpatialCompatProbeTests(unittest.TestCase):
    def test_ledger_classifies_every_maintained_case_once(self) -> None:
        regress = PROBE.load_regress_cases()
        entries = PROBE.load_ledger(PROBE.DEFAULT_LEDGER, regress)
        self.assertEqual(len(regress), 57)
        self.assertEqual(len(entries), len(regress))

    def test_ledger_rejects_missing_case(self) -> None:
        regress = PROBE.load_regress_cases()
        document = json.loads(PROBE.DEFAULT_LEDGER.read_text(encoding="utf-8"))
        document["cases"].pop()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ledger.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "coverage mismatch"):
                PROBE.load_ledger(path, regress)

    def test_wkt_normalization_is_format_insensitive(self) -> None:
        self.assertEqual(PROBE.normalize("POINT (1 2)"), PROBE.normalize("POINT(1 2)"))


if __name__ == "__main__":
    unittest.main()
