#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Build a runtime context from checksum-pinned DuckDB development artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import native_bundle

REPO_ROOT = Path(__file__).resolve().parent.parent
BUNDLE = native_bundle.load_bundle()
VERSION = BUNDLE["duckdb"]["version"]
DUCKLAKE = BUNDLE["extensions"]["ducklake"]
SPATIAL = BUNDLE["extensions"]["spatial"]
DUCKLAKE_SERIES = native_bundle.validate_series(BUNDLE, "ducklake", REPO_ROOT)
DUCKLAKE_PATCH = DUCKLAKE_SERIES["patches"][0]
EXPECTED = {
    "libduckdb.so": BUNDLE["duckdb"]["artifact"]["library_sha256"],
    "ducklake.duckdb_extension": DUCKLAKE["artifact"]["sha256"],
    "spatial.duckdb_extension": SPATIAL["artifact"]["sha256"],
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


def prepare(
    server: Path,
    migrate: Path,
    rest: Path,
    edge_bin_dir: Path,
    duckdb_bin: Path,
    ducklake_extension: Path,
    duckdb_root: Path,
    out: Path,
    *,
    allow_dirty: bool = False,
) -> dict[str, object]:
    edge_binaries = [
        edge_bin_dir / name
        for name in [
            "quackgis-bootstrap",
            "quackgis-worker-edge",
            "quackgis-client",
            "quackgis-keygen",
        ]
    ]
    if (
        not server.is_file()
        or not migrate.is_file()
        or not rest.is_file()
        or not duckdb_bin.is_file()
        or any(not binary.is_file() for binary in edge_binaries)
    ):
        raise ValueError(
            "server, migrator, REST, edge, and DuckDB CLI binaries must be regular files"
        )
    version = subprocess.run(
        [str(duckdb_bin), "--version"], text=True, capture_output=True, check=True
    ).stdout.strip()
    if not version.startswith(f"v{VERSION} "):
        raise ValueError(f"expected DuckDB v{VERSION}, got {version!r}")

    library = duckdb_root / f"v{VERSION}" / "lib" / "libduckdb.so"
    extension_dir = (
        duckdb_root / "home" / ".duckdb" / "extensions" / f"v{VERSION}" / "linux_amd64"
    )
    spatial_extension = extension_dir / "spatial.duckdb_extension"
    require_hash(library, EXPECTED[library.name])
    if ducklake_extension.is_symlink():
        raise ValueError("pinned DuckLake extension must not be a symlink")
    require_hash(
        REPO_ROOT / str(DUCKLAKE_PATCH["path"]),
        str(DUCKLAKE_PATCH["sha256"]),
    )
    require_hash(ducklake_extension, EXPECTED["ducklake.duckdb_extension"])
    require_hash(spatial_extension, EXPECTED[spatial_extension.name])

    source = git_source()
    if source["dirty"] and not allow_dirty:
        raise ValueError(
            "refusing to package a dirty worktree; commit/stash changes or pass --allow-dirty for a non-release local artifact"
        )

    if out.exists():
        shutil.rmtree(out)
    target_extensions = out / "duckdb-home" / ".duckdb" / "extensions" / f"v{VERSION}" / "linux_amd64"
    target_extensions.mkdir(parents=True)
    shutil.copy2(server, out / "quackgis-server")
    shutil.copy2(migrate, out / "quackgis-migrate")
    shutil.copy2(rest, out / "quackgis-rest")
    for binary in edge_binaries:
        shutil.copy2(binary, out / binary.name)
    shutil.copy2(duckdb_bin, out / "duckdb")
    shutil.copy2(library, out / "libduckdb.so")
    shutil.copy2(ducklake_extension, target_extensions / "ducklake.duckdb_extension")
    shutil.copy2(spatial_extension, target_extensions / spatial_extension.name)
    licenses = out / "licenses"
    licenses.mkdir()
    for name in ("LICENSE", "NOTICE", "THIRD_PARTY_LICENSES.md"):
        shutil.copy2(REPO_ROOT / name, licenses / name)

    manifest: dict[str, object] = {
        "duckdb_version": version,
        "platform": "linux-amd64",
        "source_sha": source["sha"],
        "source": source,
        "extensions": {
            "ducklake": {
                "revision": DUCKLAKE["source"]["commit"],
                "source": f"https://github.com/duckdb/ducklake/tree/{DUCKLAKE['source']['commit']}",
                "patch": DUCKLAKE_PATCH["path"],
                "patch_sha256": DUCKLAKE_PATCH["sha256"],
                "license": "MIT",
            },
            "spatial": {
                "revision": SPATIAL["source"]["commit"],
                "source": f"https://github.com/duckdb/duckdb-spatial/tree/{SPATIAL['source']['commit']}",
                "license": "MIT plus bundled third-party dependencies",
                "redistribution": "local-evaluation-only",
                "bundled_dependencies": [
                    "GEOS",
                    "GDAL",
                    "PROJ",
                    "OpenSSL",
                    "curl",
                    "expat",
                    "zlib",
                    "SQLite",
                ],
            },
        },
        "artifacts": {
            "libduckdb.so": EXPECTED["libduckdb.so"],
            "ducklake.duckdb_extension": EXPECTED["ducklake.duckdb_extension"],
            "spatial.duckdb_extension": EXPECTED["spatial.duckdb_extension"],
            "duckdb": sha256(duckdb_bin),
            "quackgis-server": sha256(server),
            "quackgis-migrate": sha256(migrate),
            "quackgis-rest": sha256(rest),
            **{binary.name: sha256(binary) for binary in edge_binaries},
            "licenses/LICENSE": sha256(licenses / "LICENSE"),
            "licenses/NOTICE": sha256(licenses / "NOTICE"),
            "licenses/THIRD_PARTY_LICENSES.md": sha256(
                licenses / "THIRD_PARTY_LICENSES.md"
            ),
        },
        "native_bundle": runtime_bundle_identity(),
        "runtime_install_allowed": False,
    }
    (out / "artifact-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def runtime_bundle_identity() -> dict[str, object]:
    components = {}
    for name in native_bundle.COMPONENTS:
        owner = BUNDLE["duckdb"] if name == "duckdb" else BUNDLE["extensions"][name]
        series = native_bundle.validate_series(BUNDLE, name, REPO_ROOT)
        components[name] = {
            "source_url": owner["source"]["url"],
            "commit": owner["source"]["commit"],
            "base_tree": owner["source"]["tree"],
            "result_tree": series["result_tree"],
            "patches": [
                {"path": patch["path"], "sha256": patch["sha256"]}
                for patch in series["patches"]
            ],
        }
    return {
        "schema_version": BUNDLE["schema_version"],
        "bundle_id": BUNDLE["bundle_id"],
        "bundle_sha256": native_bundle.canonical_sha256(BUNDLE),
        "status": BUNDLE["status"],
        "components": components,
        "toolchain": BUNDLE["toolchain"],
        "build": BUNDLE["build"],
    }


def git_source() -> dict[str, object]:
    root_result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
        text=True,
    )
    if root_result.returncode != 0 or Path(root_result.stdout.strip()).resolve() != REPO_ROOT:
        raise ValueError("cannot establish QuackGIS repository provenance")
    sha_result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, capture_output=True, check=False
    )
    status_result = subprocess.run(
        ["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
    )
    diff_result = subprocess.run(
        ["git", "diff", "--binary", "HEAD"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
    )
    untracked_result = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
    )
    if any(
        result.returncode != 0
        for result in [sha_result, status_result, diff_result, untracked_result]
    ):
        raise ValueError("cannot capture complete QuackGIS source provenance")
    status = status_result.stdout
    diff = diff_result.stdout
    dirty = bool(status)
    return {
        "sha": (
            sha_result.stdout.decode("utf-8", errors="replace").strip()
            if sha_result.returncode == 0
            else "unknown"
        ),
        "dirty": dirty,
        "status_sha256": hashlib.sha256(status).hexdigest() if dirty else None,
        "diff_sha256": hashlib.sha256(diff).hexdigest() if dirty else None,
        "source_state_sha256": (
            source_state_sha256(status, diff, untracked_result.stdout, REPO_ROOT)
            if dirty
            else None
        ),
    }


def source_state_sha256(status: bytes, diff: bytes, untracked: bytes, root: Path) -> str:
    digest = hashlib.sha256()
    digest.update(b"status\0" + status)
    digest.update(b"diff\0" + diff)
    for encoded_path in sorted(path for path in untracked.split(b"\0") if path):
        relative = Path(os.fsdecode(encoded_path))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("invalid untracked path in source provenance")
        path = root / relative
        metadata = path.lstat()
        digest.update(b"untracked\0" + encoded_path + b"\0")
        digest.update(f"{metadata.st_mode:o}".encode("ascii") + b"\0")
        if path.is_symlink():
            digest.update(b"symlink\0" + os.fsencode(path.readlink()))
        elif path.is_file():
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
        else:
            raise ValueError(f"unsupported untracked source type: {relative}")
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--migrate", type=Path, required=True)
    parser.add_argument("--rest", type=Path, required=True)
    parser.add_argument("--edge-bin-dir", type=Path, required=True)
    parser.add_argument("--duckdb-bin", type=Path, required=True)
    parser.add_argument("--ducklake-extension", type=Path, required=True)
    parser.add_argument("--duckdb-root", type=Path, default=Path(".tmp/duckdb"))
    parser.add_argument("--out", type=Path, default=Path(".tmp/duckdb-runtime"))
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="build a clearly marked non-release artifact from a dirty worktree",
    )
    args = parser.parse_args(argv)
    try:
        manifest = prepare(
            args.server.resolve(),
            args.migrate.resolve(),
            args.rest.resolve(),
            args.edge_bin_dir.resolve(),
            args.duckdb_bin.resolve(),
            args.ducklake_extension.resolve(),
            args.duckdb_root.resolve(),
            args.out.resolve(),
            allow_dirty=args.allow_dirty,
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
