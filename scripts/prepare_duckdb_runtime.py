#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Build a runtime context from checksum-pinned DuckDB development artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path


VERSION = "1.5.4"
EXPECTED = {
    "libduckdb.so": "d7f30ef2ef4b813edb94ce82906329cc689672624a4161617ea33431040ce174",
    "ducklake.duckdb_extension": "00f72402c9c5d1f69c3329f38837f4abd100cddb7c69e76650f46bf35a17babe",
    "spatial.duckdb_extension": "819a0fa94d2e8257371bb4d97f32c47586b42c85e24bab1bcbf8712a88b8ccfa",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_hash(path: Path, expected: str) -> None:
    if not path.is_file():
        raise ValueError(f"required runtime artifact is missing: {path}")
    actual = sha256(path)
    if actual != expected:
        raise ValueError(f"runtime artifact checksum mismatch for {path}: {actual}")


def prepare(server: Path, duckdb_bin: Path, duckdb_root: Path, out: Path) -> dict[str, object]:
    if not server.is_file() or not duckdb_bin.is_file():
        raise ValueError("server and DuckDB CLI binaries must be regular files")
    version = subprocess.run(
        [str(duckdb_bin), "--version"], text=True, capture_output=True, check=True
    ).stdout.strip()
    if not version.startswith(f"v{VERSION} "):
        raise ValueError(f"expected DuckDB v{VERSION}, got {version!r}")

    library = duckdb_root / f"v{VERSION}" / "lib" / "libduckdb.so"
    extension_dir = (
        duckdb_root / "home" / ".duckdb" / "extensions" / f"v{VERSION}" / "linux_amd64"
    )
    extensions = [
        extension_dir / "ducklake.duckdb_extension",
        extension_dir / "spatial.duckdb_extension",
    ]
    require_hash(library, EXPECTED[library.name])
    for extension in extensions:
        require_hash(extension, EXPECTED[extension.name])

    if out.exists():
        shutil.rmtree(out)
    target_extensions = out / "duckdb-home" / ".duckdb" / "extensions" / f"v{VERSION}" / "linux_amd64"
    target_extensions.mkdir(parents=True)
    shutil.copy2(server, out / "quackgis-server")
    shutil.copy2(duckdb_bin, out / "duckdb")
    shutil.copy2(library, out / "libduckdb.so")
    for extension in extensions:
        shutil.copy2(extension, target_extensions / extension.name)

    manifest: dict[str, object] = {
        "duckdb_version": version,
        "platform": "linux-amd64",
        "source_sha": git_sha(),
        "artifacts": {
            "libduckdb.so": EXPECTED["libduckdb.so"],
            "ducklake.duckdb_extension": EXPECTED["ducklake.duckdb_extension"],
            "spatial.duckdb_extension": EXPECTED["spatial.duckdb_extension"],
            "duckdb": sha256(duckdb_bin),
            "quackgis-server": sha256(server),
        },
        "runtime_install_allowed": False,
    }
    (out / "artifact-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def git_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], text=True, capture_output=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--duckdb-bin", type=Path, required=True)
    parser.add_argument("--duckdb-root", type=Path, default=Path(".tmp/duckdb"))
    parser.add_argument("--out", type=Path, default=Path(".tmp/duckdb-runtime"))
    args = parser.parse_args(argv)
    try:
        manifest = prepare(
            args.server.resolve(),
            args.duckdb_bin.resolve(),
            args.duckdb_root.resolve(),
            args.out.resolve(),
        )
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"prepare DuckDB runtime failed: {error}", file=sys.stderr)
        return 1
    print(
        f"duckdb_runtime_context_ok out={args.out} source_sha={manifest['source_sha']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
