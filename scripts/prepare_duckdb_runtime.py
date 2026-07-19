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
import package_native_bundle

REPO_ROOT = Path(__file__).resolve().parent.parent
BUNDLE = native_bundle.load_bundle()
VERSION = BUNDLE["duckdb"]["version"]
DUCKLAKE = BUNDLE["extensions"]["ducklake"]
SPATIAL = BUNDLE["extensions"]["spatial"]
DUCKLAKE_SERIES = native_bundle.validate_series(BUNDLE, "ducklake", REPO_ROOT)
DUCKLAKE_PATCHES = DUCKLAKE_SERIES["patches"]
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
    require_hash(duckdb_bin, BUNDLE["duckdb"]["artifact"]["cli_sha256"])
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
    for patch in DUCKLAKE_PATCHES:
        require_hash(REPO_ROOT / patch["path"], patch["sha256"])
    require_hash(ducklake_extension, EXPECTED["ducklake.duckdb_extension"])
    require_hash(spatial_extension, EXPECTED[spatial_extension.name])

    source = git_source()
    if source["dirty"] and not allow_dirty:
        raise ValueError(
            "refusing to package a dirty worktree; commit/stash changes or pass --allow-dirty for a non-release local artifact"
        )

    final_out = require_runtime_output(out)
    partial_out = require_runtime_output(final_out.with_name(f".{final_out.name}.partial"))
    if partial_out.exists() or partial_out.is_symlink():
        raise ValueError(f"remove interrupted runtime output explicitly: {partial_out}")
    partial_out.mkdir(parents=True)
    out = partial_out
    target_extensions = out / "duckdb-home" / ".duckdb" / "extensions" / f"v{VERSION}" / "linux_amd64"
    target_extensions.mkdir(parents=True)
    staged_server = out / "quackgis-server"
    staged_migrate = out / "quackgis-migrate"
    staged_rest = out / "quackgis-rest"
    staged_duckdb = out / "duckdb"
    staged_library = out / "libduckdb.so"
    staged_ducklake = target_extensions / "ducklake.duckdb_extension"
    staged_spatial = target_extensions / spatial_extension.name
    shutil.copy2(server, staged_server)
    shutil.copy2(migrate, staged_migrate)
    shutil.copy2(rest, staged_rest)
    for binary in edge_binaries:
        shutil.copy2(binary, out / binary.name)
    shutil.copy2(duckdb_bin, staged_duckdb)
    shutil.copy2(library, staged_library)
    shutil.copy2(ducklake_extension, staged_ducklake)
    shutil.copy2(spatial_extension, staged_spatial)
    staged_native_hashes = {
        "duckdb": sha256(staged_duckdb),
        "libduckdb.so": sha256(staged_library),
        "ducklake.duckdb_extension": sha256(staged_ducklake),
        "spatial.duckdb_extension": sha256(staged_spatial),
    }
    expected_native_hashes = {
        "duckdb": BUNDLE["duckdb"]["artifact"]["cli_sha256"],
        **EXPECTED,
    }
    if staged_native_hashes != expected_native_hashes:
        raise ValueError(
            "staged native runtime artifacts do not match the selected bundle: "
            f"expected {expected_native_hashes}, got {staged_native_hashes}"
        )
    licenses = out / "licenses"
    licenses.mkdir()
    for name in ("LICENSE", "NOTICE", "THIRD_PARTY_LICENSES.md"):
        shutil.copy2(REPO_ROOT / name, licenses / name)
    metadata = package_native_bundle.write_metadata(
        BUNDLE, out, context_is_unpublished=True
    )

    manifest: dict[str, object] = {
        "duckdb_version": version,
        "platform": "linux-amd64",
        "source_sha": source["sha"],
        "source": source,
        "extensions": {
            "ducklake": {
                "revision": DUCKLAKE["source"]["commit"],
                "source": f"https://github.com/duckdb/ducklake/tree/{DUCKLAKE['source']['commit']}",
                "patches": [
                    {"path": patch["path"], "sha256": patch["sha256"]}
                    for patch in DUCKLAKE_PATCHES
                ],
                "upstream_source_license": DUCKLAKE["source_license"],
                "patch_license": "NOASSERTION",
                "artifact_license": "NOASSERTION",
            },
            "spatial": {
                "revision": SPATIAL["source"]["commit"],
                "source": f"https://github.com/duckdb/duckdb-spatial/tree/{SPATIAL['source']['commit']}",
                "upstream_source_license": SPATIAL["source_license"],
                "artifact_license": "NOASSERTION",
                "redistribution": "local-evaluation-only",
                "bundled_dependencies": SPATIAL["bundled_dependencies"],
            },
        },
        "artifacts": {
            **staged_native_hashes,
            "quackgis-server": sha256(staged_server),
            "quackgis-migrate": sha256(staged_migrate),
            "quackgis-rest": sha256(staged_rest),
            **{binary.name: sha256(out / binary.name) for binary in edge_binaries},
            "licenses/LICENSE": sha256(licenses / "LICENSE"),
            "licenses/NOTICE": sha256(licenses / "NOTICE"),
            "licenses/THIRD_PARTY_LICENSES.md": sha256(
                licenses / "THIRD_PARTY_LICENSES.md"
            ),
            BUNDLE["outputs"]["sbom"]: sha256(metadata["sbom"]),
            BUNDLE["outputs"]["license_inventory"]: sha256(
                metadata["license_inventory"]
            ),
        },
        "native_bundle": runtime_bundle_identity(),
        "runtime_install_allowed": False,
    }
    (out / "artifact-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    publish_runtime_output(out, final_out)
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
        "authority_sha256": native_bundle.authority_sha256(BUNDLE, REPO_ROOT),
        "status": BUNDLE["status"],
        "upstream_review": {
            "reviewed_at": native_bundle.validate_upstream_review(BUNDLE, REPO_ROOT)[
                "reviewed_at"
            ],
            "sha256": native_bundle.file_sha256(REPO_ROOT / BUNDLE["upstream_review"]),
        },
        "components": components,
        "selected_artifacts": {
            "duckdb": {
                "source": BUNDLE["duckdb"]["source"],
                "artifact": BUNDLE["duckdb"]["artifact"],
            },
            "ducklake": {
                "source": DUCKLAKE["source"],
                "artifact": DUCKLAKE["artifact"],
            },
            "spatial": {
                "source": SPATIAL["source"],
                "artifact": SPATIAL["artifact"],
            },
        },
        "unaccepted_candidate_configuration": {
            "toolchain": BUNDLE["toolchain"],
            "build": BUNDLE["build"],
        },
    }


def require_runtime_output(path: Path) -> Path:
    temporary_root = (REPO_ROOT / ".tmp").resolve()
    lexical = Path(
        os.path.abspath(path if path.is_absolute() else REPO_ROOT / path)
    )
    try:
        relative = lexical.relative_to(temporary_root)
        resolved = lexical.resolve()
        resolved.relative_to(temporary_root)
    except ValueError as error:
        raise ValueError("runtime output must remain below workspace .tmp") from error
    if not relative.parts:
        raise ValueError("runtime output cannot be workspace .tmp itself")
    current = temporary_root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"runtime output traverses a symlink: {current}")
    return resolved


def publish_runtime_output(partial: Path, final: Path) -> None:
    require_runtime_output(partial)
    require_runtime_output(final)
    native_bundle.publish_staged_directory(partial, final)


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
    flags_result = subprocess.run(
        ["git", "ls-files", "-v"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
        text=True,
    )
    if flags_result.returncode != 0:
        raise ValueError("cannot inspect QuackGIS source index flags")
    flagged = [
        line for line in flags_result.stdout.splitlines() if not line.startswith("H ")
    ]
    if flagged:
        raise ValueError(f"QuackGIS source has non-default index flags: {flagged[:5]}")
    refresh_result = subprocess.run(
        ["git", "update-index", "--really-refresh"],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
    )
    if refresh_result.returncode not in {0, 1}:
        raise ValueError("cannot refresh QuackGIS tracked source bytes")
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
            args.out,
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
