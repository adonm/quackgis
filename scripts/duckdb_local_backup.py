#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Create and restore checksum-verified offline local DuckLake backups."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path


MANIFEST_NAME = "manifest.json"
MANIFEST_VERSION = 2
AUTHORITY_MARKER = Path("_quackgis/storage-authority-v1")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")


class BackupError(RuntimeError):
    pass


def _absolute(path: Path, *, must_exist: bool) -> Path:
    try:
        return path.expanduser().resolve(strict=must_exist)
    except OSError as error:
        raise BackupError(f"cannot resolve {path}: {error}") from error


def _require_regular(path: Path, label: str) -> None:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise BackupError(f"cannot inspect {label} {path}: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise BackupError(f"{label} must be a regular file without symlinks: {path}")


def _source_files(catalog: Path, data_root: Path) -> list[tuple[Path, Path]]:
    _require_regular(catalog, "catalog")
    if not data_root.is_dir() or data_root.is_symlink():
        raise BackupError(f"data root must be a directory without symlinks: {data_root}")
    marker = data_root / AUTHORITY_MARKER
    _require_regular(marker, "storage authority marker")

    files = [(catalog, Path("catalog.ducklake"))]
    for source in sorted(data_root.rglob("*")):
        relative = source.relative_to(data_root)
        if relative.parts and relative.parts[0] == ".tmp":
            continue
        if source.is_symlink():
            raise BackupError(f"data root contains a symlink: {source}")
        if source.is_dir():
            continue
        _require_regular(source, "data file")
        files.append((source, Path("data") / relative))
    return files


def _runtime_identity(runtime_manifest: Path) -> dict[str, object]:
    runtime_manifest = _absolute(runtime_manifest, must_exist=True)
    _require_regular(runtime_manifest, "runtime manifest")
    try:
        value = json.loads(runtime_manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BackupError(f"cannot read runtime manifest {runtime_manifest}: {error}") from error
    if not isinstance(value, dict):
        raise BackupError("runtime manifest root must be an object")

    duckdb_version = value.get("duckdb_version")
    platform = value.get("platform")
    if not isinstance(duckdb_version, str) or not duckdb_version:
        raise BackupError("runtime manifest is missing duckdb_version")
    if not isinstance(platform, str) or not platform:
        raise BackupError("runtime manifest is missing platform")

    if isinstance(value.get("libduckdb"), dict):
        library_digest = value["libduckdb"].get("sha256")
        extension_entries = value.get("extensions")
        if not isinstance(extension_entries, list):
            raise BackupError("runtime manifest extensions must be a list")
        extensions = {}
        for entry in extension_entries:
            if not isinstance(entry, dict):
                raise BackupError("runtime manifest contains an invalid extension")
            name = entry.get("name")
            digest = entry.get("sha256")
            if not isinstance(name, str) or not name:
                raise BackupError("runtime manifest extension is missing its name")
            if name in extensions:
                raise BackupError(f"runtime manifest repeats extension {name}")
            if not SHA256.fullmatch(str(digest or "")):
                raise BackupError(f"runtime manifest {name} digest must be SHA-256")
            extensions[name] = digest
    else:
        artifacts = value.get("artifacts")
        if not isinstance(artifacts, dict):
            raise BackupError("runtime manifest is missing artifacts")
        library_digest = artifacts.get("libduckdb.so")
        extensions = {
            "ducklake": artifacts.get("ducklake.duckdb_extension"),
            "spatial": artifacts.get("spatial.duckdb_extension"),
        }

    if not SHA256.fullmatch(str(library_digest or "")):
        raise BackupError("runtime manifest libduckdb digest must be SHA-256")
    for name in ("ducklake", "spatial"):
        if not SHA256.fullmatch(str(extensions.get(name) or "")):
            raise BackupError(f"runtime manifest {name} digest must be SHA-256")
    source_sha = value.get("source_sha")
    if source_sha is not None and not GIT_SHA.fullmatch(str(source_sha)):
        raise BackupError("runtime manifest source_sha must be a full Git SHA")

    identity: dict[str, object] = {
        "duckdb_version": duckdb_version,
        "platform": platform,
        "libduckdb_sha256": library_digest,
        "extensions": {name: extensions[name] for name in sorted(extensions)},
    }
    if source_sha is not None:
        identity["source_sha"] = source_sha
    return identity


def _copy_with_digest(source: Path, destination: Path) -> tuple[int, str]:
    before = source.stat()
    digest = hashlib.sha256()
    size = 0
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as reader, destination.open("xb") as writer:
        while chunk := reader.read(1024 * 1024):
            writer.write(chunk)
            digest.update(chunk)
            size += len(chunk)
        writer.flush()
        os.fsync(writer.fileno())
    after = source.stat()
    identity_before = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    identity_after = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if identity_before != identity_after or size != before.st_size:
        raise BackupError(f"source changed while it was copied: {source}")
    return size, digest.hexdigest()


def create_backup(
    catalog: Path, data_root: Path, destination: Path, runtime_manifest: Path
) -> dict[str, object]:
    catalog = _absolute(catalog, must_exist=True)
    data_root = _absolute(data_root, must_exist=True)
    destination = _absolute(destination, must_exist=False)
    if destination.exists():
        raise BackupError(f"backup destination already exists: {destination}")
    if destination.is_relative_to(data_root):
        raise BackupError("backup destination cannot be inside the DuckLake data root")
    if not destination.parent.is_dir():
        raise BackupError(f"backup destination parent does not exist: {destination.parent}")
    runtime = _runtime_identity(runtime_manifest)

    staging = destination.with_name(f".{destination.name}.tmp-{uuid.uuid4().hex}")
    entries: list[dict[str, object]] = []
    try:
        staging.mkdir(mode=0o700)
        for source, relative in _source_files(catalog, data_root):
            size, digest = _copy_with_digest(source, staging / relative)
            entries.append({"path": relative.as_posix(), "size": size, "sha256": digest})
        manifest: dict[str, object] = {
            "format": "quackgis-local-backup",
            "version": MANIFEST_VERSION,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "source_catalog": str(catalog),
            "source_data_root": str(data_root),
            "runtime": runtime,
            "files": entries,
        }
        manifest_path = staging / MANIFEST_NAME
        with manifest_path.open("x", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        staging.rename(destination)
        return manifest
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def _load_and_verify(backup: Path) -> dict[str, object]:
    backup = _absolute(backup, must_exist=True)
    if not backup.is_dir() or backup.is_symlink():
        raise BackupError(f"backup must be a directory without symlinks: {backup}")
    try:
        manifest = json.loads((backup / MANIFEST_NAME).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BackupError(f"cannot read backup manifest: {error}") from error
    if (
        manifest.get("format") != "quackgis-local-backup"
        or manifest.get("version") != MANIFEST_VERSION
    ):
        raise BackupError("unsupported local backup manifest")
    if not isinstance(manifest.get("runtime"), dict):
        raise BackupError("backup manifest has no runtime identity")
    entries = manifest.get("files")
    if not isinstance(entries, list) or not entries:
        raise BackupError("backup manifest has no files")

    expected = {MANIFEST_NAME}
    for entry in entries:
        if not isinstance(entry, dict):
            raise BackupError("backup manifest contains an invalid file entry")
        relative = Path(str(entry.get("path", "")))
        if relative.is_absolute() or ".." in relative.parts or not relative.parts:
            raise BackupError(f"backup manifest contains an unsafe path: {relative}")
        expected.add(relative.as_posix())
        source = backup / relative
        _require_regular(source, "backup file")
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        if source.stat().st_size != entry.get("size") or digest != entry.get("sha256"):
            raise BackupError(f"backup checksum mismatch: {relative.as_posix()}")

    actual = set()
    for path in backup.rglob("*"):
        if path.is_symlink():
            raise BackupError(f"backup contains a symlink: {path}")
        if path.is_file():
            actual.add(path.relative_to(backup).as_posix())
    if actual != expected:
        raise BackupError(
            f"backup file set differs from manifest: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )
    return manifest


def restore_backup(
    backup: Path, catalog: Path, data_root: Path, runtime_manifest: Path
) -> dict[str, object]:
    backup = _absolute(backup, must_exist=True)
    manifest = _load_and_verify(backup)
    if manifest["runtime"] != _runtime_identity(runtime_manifest):
        raise BackupError("backup runtime identity does not match the selected runtime")
    catalog = _absolute(catalog, must_exist=False)
    data_root = _absolute(data_root, must_exist=False)
    if str(catalog) != manifest.get("source_catalog") or str(data_root) != manifest.get(
        "source_data_root"
    ):
        raise BackupError("local restore must target the exact original catalog and data paths")
    if catalog.exists() or data_root.exists():
        raise BackupError("restore targets must not already exist")
    if not catalog.parent.is_dir() or not data_root.parent.is_dir():
        raise BackupError("restore target parents must already exist")

    token = uuid.uuid4().hex
    catalog_staging = catalog.with_name(f".{catalog.name}.restore-{token}")
    data_staging = data_root.with_name(f".{data_root.name}.restore-{token}")
    try:
        data_staging.mkdir(mode=0o700)
        for entry in manifest["files"]:
            relative = Path(str(entry["path"]))
            source = backup / relative
            if relative == Path("catalog.ducklake"):
                destination = catalog_staging
            else:
                destination = data_staging / relative.relative_to("data")
            size, digest = _copy_with_digest(source, destination)
            if size != entry["size"] or digest != entry["sha256"]:
                raise BackupError(f"restored checksum mismatch: {relative.as_posix()}")
        data_staging.rename(data_root)
        try:
            catalog_staging.rename(catalog)
        except Exception:
            shutil.rmtree(data_root, ignore_errors=True)
            raise
        return manifest
    except Exception:
        catalog_staging.unlink(missing_ok=True)
        shutil.rmtree(data_staging, ignore_errors=True)
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    backup = subparsers.add_parser("backup")
    backup.add_argument("--catalog", type=Path, required=True)
    backup.add_argument("--data-root", type=Path, required=True)
    backup.add_argument("--destination", type=Path, required=True)
    backup.add_argument("--runtime-manifest", type=Path, required=True)
    restore = subparsers.add_parser("restore")
    restore.add_argument("--backup", type=Path, required=True)
    restore.add_argument("--catalog", type=Path, required=True)
    restore.add_argument("--data-root", type=Path, required=True)
    restore.add_argument("--runtime-manifest", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    arguments = parse_args(argv)
    try:
        if arguments.command == "backup":
            manifest = create_backup(
                arguments.catalog,
                arguments.data_root,
                arguments.destination,
                arguments.runtime_manifest,
            )
        else:
            manifest = restore_backup(
                arguments.backup,
                arguments.catalog,
                arguments.data_root,
                arguments.runtime_manifest,
            )
    except (BackupError, OSError, ValueError) as error:
        print(f"duckdb_local_backup_error: {error}", file=sys.stderr)
        return 1
    print(
        f"duckdb_local_{arguments.command}_ok files={len(manifest['files'])} "
        f"created_at={manifest['created_at']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
