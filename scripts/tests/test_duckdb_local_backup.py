#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "duckdb_local_backup", ROOT / "scripts/duckdb_local_backup.py"
)
assert SPEC and SPEC.loader
BACKUP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BACKUP)


class DuckDbLocalBackupTests(unittest.TestCase):
    def fixture(self, root: Path) -> tuple[Path, Path]:
        catalog = root / "catalog.ducklake"
        catalog.write_bytes(b"catalog-v1")
        data = root / "data"
        (data / "_quackgis").mkdir(parents=True)
        (data / "_quackgis/storage-authority-v1").write_text(
            "quackgis-duckdb-official-ducklake-v1\n", encoding="utf-8"
        )
        (data / "main").mkdir()
        (data / "main/part-1.parquet").write_bytes(b"PAR1payload")
        (data / ".tmp").mkdir()
        (data / ".tmp/spill.bin").write_bytes(b"scratch")
        return catalog, data

    def runtime_manifest(self, root: Path, *, spatial: str = "c" * 64) -> Path:
        manifest = root / f"runtime-{spatial[:8]}.json"
        manifest.write_text(
            "{\n"
            '  "duckdb_version": "1.5.4",\n'
            '  "platform": "linux-amd64",\n'
            f'  "libduckdb": {{"sha256": "{"a" * 64}"}},\n'
            '  "extensions": [\n'
            f'    {{"name": "ducklake", "sha256": "{"b" * 64}"}},\n'
            f'    {{"name": "spatial", "sha256": "{spatial}"}}\n'
            "  ]\n"
            "}\n",
            encoding="utf-8",
        )
        return manifest

    def test_roundtrip_is_exact_path_and_excludes_spill(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalog, data = self.fixture(root)
            runtime = self.runtime_manifest(root)
            destination = root / "backup"
            manifest = BACKUP.create_backup(catalog, data, destination, runtime)
            paths = {entry["path"] for entry in manifest["files"]}
            self.assertIn("catalog.ducklake", paths)
            self.assertIn("data/_quackgis/storage-authority-v1", paths)
            self.assertNotIn("data/.tmp/spill.bin", paths)

            catalog.unlink()
            for path in sorted(data.rglob("*"), reverse=True):
                path.unlink() if path.is_file() else path.rmdir()
            data.rmdir()
            BACKUP.restore_backup(destination, catalog, data, runtime)
            self.assertEqual(catalog.read_bytes(), b"catalog-v1")
            self.assertEqual((data / "main/part-1.parquet").read_bytes(), b"PAR1payload")
            self.assertFalse((data / ".tmp").exists())

            with self.assertRaisesRegex(BACKUP.BackupError, "must not already exist"):
                BACKUP.restore_backup(destination, catalog, data, runtime)

    def test_tampering_and_relocation_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalog, data = self.fixture(root)
            runtime = self.runtime_manifest(root)
            destination = root / "backup"
            BACKUP.create_backup(catalog, data, destination, runtime)
            with self.assertRaisesRegex(BACKUP.BackupError, "exact original"):
                BACKUP.restore_backup(
                    destination,
                    root / "other.ducklake",
                    root / "other-data",
                    runtime,
                )

            catalog.unlink()
            for path in sorted(data.rglob("*"), reverse=True):
                path.unlink() if path.is_file() else path.rmdir()
            data.rmdir()
            (destination / "catalog.ducklake").write_bytes(b"tampered")
            with self.assertRaisesRegex(BACKUP.BackupError, "checksum mismatch"):
                BACKUP.restore_backup(destination, catalog, data, runtime)
            self.assertFalse(catalog.exists())
            self.assertFalse(data.exists())

    def test_restore_rejects_a_different_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalog, data = self.fixture(root)
            runtime = self.runtime_manifest(root)
            different_runtime = self.runtime_manifest(root, spatial="d" * 64)
            destination = root / "backup"
            BACKUP.create_backup(catalog, data, destination, runtime)
            catalog.unlink()
            for path in sorted(data.rglob("*"), reverse=True):
                path.unlink() if path.is_file() else path.rmdir()
            data.rmdir()

            with self.assertRaisesRegex(BACKUP.BackupError, "runtime identity"):
                BACKUP.restore_backup(
                    destination, catalog, data, different_runtime
                )
            self.assertFalse(catalog.exists())
            self.assertFalse(data.exists())

    def test_release_artifact_manifest_has_the_same_bounded_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "artifact-manifest.json"
            manifest.write_text(
                "{\n"
                '  "duckdb_version": "1.5.4",\n'
                '  "platform": "linux-amd64",\n'
                f'  "source_sha": "{"e" * 40}",\n'
                '  "artifacts": {\n'
                f'    "libduckdb.so": "{"a" * 64}",\n'
                f'    "ducklake.duckdb_extension": "{"b" * 64}",\n'
                f'    "spatial.duckdb_extension": "{"c" * 64}"\n'
                "  }\n"
                "}\n",
                encoding="utf-8",
            )
            identity = BACKUP._runtime_identity(manifest)
            self.assertEqual(identity["source_sha"], "e" * 40)
            self.assertEqual(identity["libduckdb_sha256"], "a" * 64)
            self.assertEqual(
                identity["extensions"],
                {"ducklake": "b" * 64, "spatial": "c" * 64},
            )

    def test_source_symlinks_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalog, data = self.fixture(root)
            (data / "linked.parquet").symlink_to(data / "main/part-1.parquet")
            with self.assertRaisesRegex(BACKUP.BackupError, "symlink"):
                BACKUP.create_backup(
                    catalog, data, root / "backup", self.runtime_manifest(root)
                )


if __name__ == "__main__":
    unittest.main()
