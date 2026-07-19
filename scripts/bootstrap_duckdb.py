#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Install the pinned DuckDB ADBC library and official extensions locally."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

import native_bundle


ROOT = Path(__file__).resolve().parent.parent
BUNDLE = native_bundle.load_bundle()
DUCKDB = BUNDLE["duckdb"]
VERSION = str(DUCKDB["version"])
PLATFORM = str(BUNDLE["platform"])
ARCHIVE_NAME = f"libduckdb-{PLATFORM}.zip"
ARCHIVE_URL = str(DUCKDB["artifact"]["archive_url"])
ARCHIVE_SHA256 = str(DUCKDB["artifact"]["archive_sha256"])
LIBRARY_SHA256 = str(DUCKDB["artifact"]["library_sha256"])
CLI_ARCHIVE_URL = str(DUCKDB["artifact"]["cli_archive_url"])
CLI_ARCHIVE_SHA256 = str(DUCKDB["artifact"]["cli_archive_sha256"])
CLI_SHA256 = str(DUCKDB["artifact"]["cli_sha256"])
ARCHIVE_MEMBERS = ("libduckdb.so", "duckdb.h", "duckdb.hpp")
EXTENSIONS = ("ducklake", "spatial")
EXTENSION_SHA256 = {
    name: str(DUCKDB["artifact"]["official_extension_sha256"][name])
    for name in EXTENSIONS
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_platform() -> None:
    machine = platform.machine().lower()
    if sys.platform != "linux" or machine not in {"x86_64", "amd64"}:
        raise RuntimeError(
            "the pinned libduckdb bootstrap currently supports Linux x86_64 only; "
            f"got {sys.platform}/{machine}"
        )


def require_workspace_root(path: Path) -> Path:
    temporary_root = (ROOT / ".tmp").resolve()
    lexical = Path(os.path.abspath(path if path.is_absolute() else ROOT / path))
    try:
        relative = lexical.relative_to(temporary_root)
        resolved = lexical.resolve()
        resolved.relative_to(temporary_root)
    except ValueError as error:
        raise RuntimeError("DuckDB bootstrap root must remain below workspace .tmp") from error
    if not relative.parts:
        raise RuntimeError("DuckDB bootstrap root cannot be workspace .tmp itself")
    current = temporary_root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise RuntimeError(f"DuckDB bootstrap root traverses a symlink: {current}")
    return resolved


def require_owned_path(path: Path, root: Path, label: str) -> None:
    try:
        relative = path.relative_to(root)
        path.resolve().relative_to(root.resolve())
    except ValueError as error:
        raise RuntimeError(f"{label} escapes the DuckDB bootstrap root") from error
    current = root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise RuntimeError(f"{label} traverses a symlink: {current}")


def write_json(path: Path, value: dict[str, object], root: Path) -> None:
    require_owned_path(path, root, "DuckDB bootstrap manifest")
    if path.is_symlink():
        raise RuntimeError("DuckDB bootstrap manifest cannot be a symlink")
    partial = path.with_name(f".{path.name}.partial")
    if partial.exists() or partial.is_symlink():
        raise RuntimeError("remove interrupted DuckDB bootstrap manifest explicitly")
    try:
        with partial.open("x", encoding="utf-8") as output:
            output.write(json.dumps(value, indent=2) + "\n")
        partial.replace(path)
    finally:
        if partial.exists() and not partial.is_symlink():
            partial.unlink()


def prepare_home(root: Path) -> Path:
    home = root / "home"
    require_owned_path(home, root, "DuckDB bootstrap home")
    if home.exists():
        if home.is_symlink() or not home.is_dir():
            raise RuntimeError("DuckDB bootstrap home must be a non-symlink directory")
        shutil.rmtree(home)
    home.mkdir()
    return home


def download_archive(path: Path) -> None:
    if path.is_symlink():
        raise RuntimeError("libduckdb archive cannot be a symlink")
    if path.is_file() and sha256(path) == ARCHIVE_SHA256:
        return
    path.unlink(missing_ok=True)
    partial = path.with_suffix(path.suffix + ".part")
    if partial.is_symlink():
        raise RuntimeError("libduckdb partial archive cannot be a symlink")
    partial.unlink(missing_ok=True)
    print(f"downloading {ARCHIVE_URL}")
    try:
        with urllib.request.urlopen(ARCHIVE_URL, timeout=60) as response, partial.open(
            "wb"
        ) as output:
            shutil.copyfileobj(response, output)
        actual = sha256(partial)
        if actual != ARCHIVE_SHA256:
            raise RuntimeError(
                f"libduckdb archive checksum mismatch: expected {ARCHIVE_SHA256}, got {actual}"
            )
        partial.replace(path)
    finally:
        partial.unlink(missing_ok=True)


def extract_library(archive: Path, destination: Path, root: Path) -> Path:
    require_owned_path(destination, root, "libduckdb extraction directory")
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as source:
        names = set(source.namelist())
        missing = set(ARCHIVE_MEMBERS) - names
        if missing:
            raise RuntimeError(f"libduckdb archive is missing: {sorted(missing)}")
        for name in ARCHIVE_MEMBERS:
            target = destination / name
            partial = target.with_suffix(target.suffix + ".part")
            require_owned_path(target, root, f"libduckdb member {name}")
            if target.is_symlink() or partial.is_symlink():
                raise RuntimeError(f"libduckdb member output cannot be a symlink: {name}")
            partial.unlink(missing_ok=True)
            with source.open(name) as input_file, partial.open("wb") as output:
                shutil.copyfileobj(input_file, output)
            partial.replace(target)
    library = destination / "libduckdb.so"
    actual = sha256(library)
    if actual != LIBRARY_SHA256:
        raise RuntimeError(
            f"libduckdb shared-library checksum mismatch: expected {LIBRARY_SHA256}, got {actual}"
        )
    return library


def require_cli(duckdb_bin: str) -> tuple[Path, str]:
    binary = shutil.which(duckdb_bin) if "/" not in duckdb_bin else duckdb_bin
    if not binary or not Path(binary).is_file():
        raise RuntimeError("DuckDB CLI is unavailable; run `mise install` first")
    binary_path = Path(binary).resolve()
    actual_cli_sha256 = sha256(binary_path)
    if actual_cli_sha256 != CLI_SHA256:
        raise RuntimeError(
            f"DuckDB CLI checksum mismatch: expected {CLI_SHA256}, got {actual_cli_sha256}"
        )
    return binary_path, actual_cli_sha256


def install_extensions(
    duckdb_bin: str, home: Path
) -> tuple[str, dict[str, str], list[dict[str, str]]]:
    binary_path, actual_cli_sha256 = require_cli(duckdb_bin)
    version = subprocess.run(
        [str(binary_path), "--version"], text=True, capture_output=True, check=True
    ).stdout.strip()
    if not version.startswith(f"v{VERSION} "):
        raise RuntimeError(f"expected DuckDB v{VERSION}, got {version!r}")

    home.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "HOME": str(home.resolve())}
    sql = "; ".join(f"INSTALL {extension}" for extension in EXTENSIONS)
    subprocess.run(
        [str(binary_path), "-batch", ":memory:", "-c", sql + ";"], env=env, check=True
    )

    extension_root = home / ".duckdb" / "extensions" / f"v{VERSION}"
    installed: list[dict[str, str]] = []
    for extension in EXTENSIONS:
        matches = sorted(extension_root.glob(f"*/{extension}.duckdb_extension"))
        if len(matches) != 1:
            raise RuntimeError(
                f"expected one installed {extension} extension under {extension_root}, "
                f"found {len(matches)}"
            )
        if matches[0].is_symlink():
            raise RuntimeError(f"installed {extension} extension cannot be a symlink")
        installed.append(
            {
                "name": extension,
                "path": str(matches[0].resolve()),
                "sha256": sha256(matches[0]),
            }
        )
        if installed[-1]["sha256"] != EXTENSION_SHA256[extension]:
            raise RuntimeError(
                f"installed {extension} checksum mismatch: expected "
                f"{EXTENSION_SHA256[extension]}, got {installed[-1]['sha256']}"
            )

    load_sql = "; ".join(f"LOAD {extension}" for extension in EXTENSIONS)
    subprocess.run(
        [str(binary_path), "-batch", ":memory:", "-c", load_sql + "; SELECT 1;"],
        env=env,
        check=True,
    )
    return (
        version,
        {
            "archive_url": CLI_ARCHIVE_URL,
            "archive_sha256": CLI_ARCHIVE_SHA256,
            "path": str(binary_path),
            "sha256": actual_cli_sha256,
        },
        installed,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(".tmp/duckdb"))
    parser.add_argument("--duckdb-bin", default="duckdb")
    args = parser.parse_args(argv)

    try:
        require_platform()
        root = require_workspace_root(args.root)
        staged_root = require_workspace_root(root.with_name(f".{root.name}.partial"))
        if staged_root.exists() or staged_root.is_symlink():
            raise RuntimeError(f"remove interrupted DuckDB bootstrap explicitly: {staged_root}")
        staged_root.mkdir(parents=True)
        downloads = staged_root / "downloads"
        require_owned_path(downloads, staged_root, "DuckDB download directory")
        downloads.mkdir(parents=True, exist_ok=True)
        archive = downloads / ARCHIVE_NAME
        download_archive(archive)
        library = extract_library(
            archive, staged_root / f"v{VERSION}" / "lib", staged_root
        )
        cli_version, cli, extensions = install_extensions(
            args.duckdb_bin, prepare_home(staged_root)
        )
        published_library = root / library.relative_to(staged_root)
        published_extensions = [
            {
                **extension,
                "path": str(root / Path(extension["path"]).relative_to(staged_root)),
            }
            for extension in extensions
        ]
        manifest = {
            "duckdb_version": VERSION,
            "cli_version": cli_version,
            "cli": cli,
            "platform": PLATFORM,
            "libduckdb": {
                "archive_url": ARCHIVE_URL,
                "archive_sha256": ARCHIVE_SHA256,
                "path": str(published_library),
                "sha256": sha256(library),
            },
            "extensions": published_extensions,
        }
        staged_manifest = staged_root / "manifest.json"
        write_json(staged_manifest, manifest, staged_root)
        native_bundle.publish_staged_directory(staged_root, root)
        manifest_path = root / "manifest.json"
        print(f"duckdb_bootstrap_ok manifest={manifest_path} driver={published_library}")
        return 0
    except (OSError, RuntimeError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"duckdb bootstrap failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
