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


VERSION = "1.5.4"
PLATFORM = "linux-amd64"
ARCHIVE_NAME = f"libduckdb-{PLATFORM}.zip"
ARCHIVE_URL = (
    f"https://github.com/duckdb/duckdb/releases/download/v{VERSION}/{ARCHIVE_NAME}"
)
ARCHIVE_SHA256 = "838d98a85e697bab9935010c88a8c67d3312ccedcab4cb4a0ba01da65113bb70"
LIBRARY_SHA256 = "d7f30ef2ef4b813edb94ce82906329cc689672624a4161617ea33431040ce174"
ARCHIVE_MEMBERS = ("libduckdb.so", "duckdb.h", "duckdb.hpp")
EXTENSIONS = ("ducklake", "spatial")


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


def download_archive(path: Path) -> None:
    if path.is_file() and sha256(path) == ARCHIVE_SHA256:
        return
    path.unlink(missing_ok=True)
    partial = path.with_suffix(path.suffix + ".part")
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


def extract_library(archive: Path, destination: Path) -> Path:
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as source:
        names = set(source.namelist())
        missing = set(ARCHIVE_MEMBERS) - names
        if missing:
            raise RuntimeError(f"libduckdb archive is missing: {sorted(missing)}")
        for name in ARCHIVE_MEMBERS:
            target = destination / name
            partial = target.with_suffix(target.suffix + ".part")
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


def install_extensions(duckdb_bin: str, home: Path) -> tuple[str, list[dict[str, str]]]:
    binary = shutil.which(duckdb_bin) if "/" not in duckdb_bin else duckdb_bin
    if not binary or not Path(binary).is_file():
        raise RuntimeError("DuckDB CLI is unavailable; run `mise install` first")
    version = subprocess.run(
        [binary, "--version"], text=True, capture_output=True, check=True
    ).stdout.strip()
    if not version.startswith(f"v{VERSION} "):
        raise RuntimeError(f"expected DuckDB v{VERSION}, got {version!r}")

    home.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "HOME": str(home.resolve())}
    sql = "; ".join(f"INSTALL {extension}" for extension in EXTENSIONS)
    subprocess.run(
        [binary, "-batch", ":memory:", "-c", sql + ";"], env=env, check=True
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
        installed.append(
            {
                "name": extension,
                "path": str(matches[0].resolve()),
                "sha256": sha256(matches[0]),
            }
        )

    load_sql = "; ".join(f"LOAD {extension}" for extension in EXTENSIONS)
    subprocess.run(
        [binary, "-batch", ":memory:", "-c", load_sql + "; SELECT 1;"],
        env=env,
        check=True,
    )
    return version, installed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(".tmp/duckdb"))
    parser.add_argument("--duckdb-bin", default="duckdb")
    args = parser.parse_args(argv)

    try:
        require_platform()
        root = args.root.resolve()
        downloads = root / "downloads"
        downloads.mkdir(parents=True, exist_ok=True)
        archive = downloads / ARCHIVE_NAME
        download_archive(archive)
        library = extract_library(archive, root / f"v{VERSION}" / "lib")
        cli_version, extensions = install_extensions(args.duckdb_bin, root / "home")
        manifest = {
            "duckdb_version": VERSION,
            "cli_version": cli_version,
            "platform": PLATFORM,
            "libduckdb": {
                "archive_url": ARCHIVE_URL,
                "archive_sha256": ARCHIVE_SHA256,
                "path": str(library),
                "sha256": sha256(library),
            },
            "extensions": extensions,
        }
        manifest_path = root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        print(f"duckdb_bootstrap_ok manifest={manifest_path} driver={library}")
        return 0
    except (OSError, RuntimeError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"duckdb bootstrap failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
