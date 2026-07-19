#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate deterministic native bundle SPDX and license metadata."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any

import native_bundle


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / ".tmp/native-bundle-metadata"


def source_download(source: dict[str, str]) -> str:
    return f"{source['url'].removesuffix('.git')}/tree/{source['commit']}"


def spdx_binary_package(
    name: str,
    version: str,
    artifact_sha256: str,
    declared_license: str,
    concluded_license: str,
    comment: str,
) -> dict[str, Any]:
    return {
        "SPDXID": f"SPDXRef-Package-{name}",
        "name": name,
        "versionInfo": version,
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": False,
        "licenseConcluded": concluded_license,
        "licenseDeclared": declared_license,
        "copyrightText": "NOASSERTION",
        "checksums": [{"algorithm": "SHA256", "checksumValue": artifact_sha256}],
        "packageComment": comment,
    }


def spdx_source_package(
    name: str, source: dict[str, str], declared_license: str
) -> dict[str, Any]:
    return {
        "SPDXID": f"SPDXRef-Package-{name}-source",
        "name": f"{name}-source",
        "versionInfo": source["commit"],
        "downloadLocation": source_download(source),
        "filesAnalyzed": False,
        "licenseConcluded": declared_license,
        "licenseDeclared": declared_license,
        "copyrightText": "NOASSERTION",
        "packageComment": f"Exact upstream Git source at tree {source['tree']}.",
    }


def patch_records(bundle: dict[str, Any]) -> list[dict[str, Any]]:
    records = []
    binaries = {
        "duckdb": ("DuckDB", "DuckDB-CLI"),
        "ducklake": ("DuckLake",),
        "spatial": ("DuckDB-Spatial",),
    }
    for component in native_bundle.COMPONENTS:
        series = native_bundle.validate_series(bundle, component, ROOT)
        for index, patch in enumerate(series["patches"], start=1):
            identifier = f"QuackGIS-{component}-patch-{index}"
            records.append(
                {
                    "component": component,
                    "identifier": identifier,
                    "spdx_id": f"SPDXRef-Package-{identifier}",
                    "binaries": binaries[component],
                    "patch": patch,
                }
            )
    return records


def spdx_document(bundle: dict[str, Any]) -> dict[str, Any]:
    authority_sha256 = native_bundle.authority_sha256(bundle, ROOT)
    duckdb = bundle["duckdb"]
    ducklake = bundle["extensions"]["ducklake"]
    spatial = bundle["extensions"]["spatial"]
    patches = patch_records(bundle)
    packages = [
        spdx_binary_package(
            "DuckDB",
            duckdb["version"],
            duckdb["artifact"]["library_sha256"],
            "NOASSERTION",
            "NOASSERTION",
            f"Selected {duckdb['artifact']['mode']} libduckdb artifact for {bundle['platform']}; "
            f"contained in {duckdb['artifact']['archive_url']} with archive SHA-256 "
            f"{duckdb['artifact']['archive_sha256']}.",
        ),
        spdx_binary_package(
            "DuckDB-CLI",
            duckdb["version"],
            duckdb["artifact"]["cli_sha256"],
            "NOASSERTION",
            "NOASSERTION",
            f"Selected vendor-built DuckDB CLI for {bundle['platform']}; "
            f"contained in {duckdb['artifact']['cli_archive_url']} with archive SHA-256 "
            f"{duckdb['artifact']['cli_archive_sha256']}.",
        ),
        spdx_binary_package(
            "DuckLake",
            ducklake["source"]["commit"],
            ducklake["artifact"]["sha256"],
            "NOASSERTION",
            "NOASSERTION",
            "Project-built extension includes the digest-pinned QuackGIS identity patch; complete concluded-license review remains required.",
        ),
        spdx_binary_package(
            "DuckDB-Spatial",
            spatial["source"]["commit"],
            spatial["artifact"]["sha256"],
            "NOASSERTION",
            "NOASSERTION",
            "Vendor-built signed extension bundles native dependencies whose exact versions and concluded licenses remain release-blocking.",
        ),
        spdx_source_package("DuckDB", duckdb["source"], duckdb["source_license"]),
        spdx_source_package(
            "DuckLake", ducklake["source"], ducklake["source_license"]
        ),
        spdx_source_package(
            "DuckDB-Spatial", spatial["source"], spatial["source_license"]
        ),
    ] + [
        {
            "SPDXID": record["spdx_id"],
            "name": record["identifier"],
            "versionInfo": record["patch"]["sha256"],
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
            "checksums": [
                {
                    "algorithm": "SHA256",
                    "checksumValue": record["patch"]["sha256"],
                }
            ],
            "packageComment": (
                f"Tracked QuackGIS source patch {record['patch']['path']} applied to "
                f"the upstream {record['component']} source package; explicit declared "
                "and concluded license review remains release-blocking."
            ),
        }
        for record in patches
    ]
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": bundle["bundle_id"],
        "documentNamespace": (
            "https://quackgis.invalid/spdx/"
            f"{bundle['bundle_id']}/{authority_sha256}"
        ),
        "creationInfo": {
            "created": bundle["outputs"]["spdx_created"],
            "creators": [
                "Organization: QuackGIS contributors",
                "Tool: scripts/package_native_bundle.py",
            ],
        },
        "documentDescribes": [package["SPDXID"] for package in packages],
        "packages": packages,
        "relationships": [
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": package["SPDXID"],
            }
            for package in packages
        ]
        + [
            {
                "spdxElementId": f"SPDXRef-Package-{binary}",
                "relationshipType": "GENERATED_FROM",
                "relatedSpdxElement": f"SPDXRef-Package-{source}-source",
            }
            for binary, source in (
                ("DuckDB", "DuckDB"),
                ("DuckDB-CLI", "DuckDB"),
                ("DuckLake", "DuckLake"),
                ("DuckDB-Spatial", "DuckDB-Spatial"),
            )
        ]
        + [
            {
                "spdxElementId": f"SPDXRef-Package-{binary}",
                "relationshipType": "GENERATED_FROM",
                "relatedSpdxElement": record["spdx_id"],
            }
            for record in patches
            for binary in record["binaries"]
        ],
    }


def license_inventory(bundle: dict[str, Any]) -> dict[str, Any]:
    duckdb = bundle["duckdb"]
    ducklake = bundle["extensions"]["ducklake"]
    spatial = bundle["extensions"]["spatial"]
    patches = patch_records(bundle)
    patch_inventory = {
        component: [
            {
                "path": record["patch"]["path"],
                "sha256": record["patch"]["sha256"],
                "license_declared": "NOASSERTION",
                "license_concluded": "NOASSERTION",
            }
            for record in patches
            if record["component"] == component
        ]
        for component in native_bundle.COMPONENTS
    }
    return {
        "schema_version": 1,
        "bundle_id": bundle["bundle_id"],
        "bundle_sha256": native_bundle.canonical_sha256(bundle),
        "authority_sha256": native_bundle.authority_sha256(bundle, ROOT),
        "complete": False,
        "redistribution_status": "local-evaluation-only",
        "components": [
            {
                "name": "duckdb",
                "source_url": duckdb["source"]["url"],
                "commit": duckdb["source"]["commit"],
                "upstream_source_license_declared": duckdb["source_license"],
                "license_declared": "NOASSERTION",
                "license_concluded": "NOASSERTION",
                "artifact_sha256": duckdb["artifact"]["library_sha256"],
                "cli_sha256": duckdb["artifact"]["cli_sha256"],
                "patches": patch_inventory["duckdb"],
            },
            {
                "name": "ducklake",
                "source_url": ducklake["source"]["url"],
                "commit": ducklake["source"]["commit"],
                "upstream_source_license_declared": ducklake["source_license"],
                "license_declared": "NOASSERTION",
                "license_concluded": "NOASSERTION",
                "artifact_sha256": ducklake["artifact"]["sha256"],
                "build_provenance": ducklake["artifact"]["build_provenance"],
                "patches": patch_inventory["ducklake"],
            },
            {
                "name": "spatial",
                "source_url": spatial["source"]["url"],
                "commit": spatial["source"]["commit"],
                "upstream_source_license_declared": spatial["source_license"],
                "license_declared": "NOASSERTION",
                "license_concluded": "NOASSERTION",
                "artifact_sha256": spatial["artifact"]["sha256"],
                "patches": patch_inventory["spatial"],
            },
        ],
        "unresolved": [
            {
                "component": "DuckDB-selected-CLI-and-library",
                "bundled_by": "duckdb",
                "missing": [
                    "bundled_component_inventory",
                    "license_concluded",
                    "notice_materials",
                ],
                "release_blocking": True,
            }
        ]
        + [
            {
                "component": record["identifier"],
                "bundled_by": record["component"],
                "missing": ["license_declared", "license_concluded"],
                "release_blocking": True,
            }
            for record in patches
        ]
        + [
            {
                "component": dependency,
                "bundled_by": "spatial",
                "missing": [
                    "exact_version",
                    "source_archive",
                    "license_concluded",
                    "notice_or_relinking_materials",
                ],
                "release_blocking": True,
            }
            for dependency in spatial["bundled_dependencies"]
        ],
    }


def require_owned_output(path: Path, out: Path, label: str) -> None:
    try:
        relative = path.relative_to(out)
        path.resolve().relative_to(out.resolve())
    except ValueError as error:
        raise ValueError(f"{label} escapes native metadata output") from error
    current = out
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"{label} traverses a symlink: {current}")


def write_json(path: Path, value: dict[str, Any], out: Path) -> None:
    require_owned_output(path, out, "native metadata file")
    path.parent.mkdir(parents=True, exist_ok=True)
    require_owned_output(path, out, "native metadata file")
    if path.is_symlink():
        raise ValueError("native metadata file cannot be a symlink")
    partial = path.with_name(f".{path.name}.partial")
    if partial.exists() or partial.is_symlink():
        raise ValueError("remove interrupted native metadata output explicitly")
    try:
        with partial.open("x", encoding="utf-8") as output:
            output.write(json.dumps(value, indent=2, sort_keys=True) + "\n")
        partial.replace(path)
    finally:
        if partial.exists() and not partial.is_symlink():
            partial.unlink()


def write_metadata(
    bundle: dict[str, Any],
    out: Path,
    *,
    context_is_unpublished: bool = False,
) -> dict[str, Path]:
    out = require_workspace_output(out)
    if context_is_unpublished:
        out.mkdir(parents=True, exist_ok=True)
        target = out
    else:
        target = require_workspace_output(out.with_name(f".{out.name}.metadata.partial"))
        if target.exists() or target.is_symlink():
            raise ValueError(f"remove interrupted native metadata output explicitly: {target}")
        target.mkdir(parents=True)
    outputs = bundle["outputs"]
    staged_paths = {
        "sbom": target / outputs["sbom"],
        "license_inventory": target / outputs["license_inventory"],
    }
    write_json(staged_paths["sbom"], spdx_document(bundle), target)
    write_json(staged_paths["license_inventory"], license_inventory(bundle), target)
    if context_is_unpublished:
        return staged_paths
    native_bundle.publish_staged_directory(target, out)
    return {
        "sbom": out / outputs["sbom"],
        "license_inventory": out / outputs["license_inventory"],
    }


def require_workspace_output(path: Path) -> Path:
    temporary_root = (ROOT / ".tmp").resolve()
    lexical = Path(os.path.abspath(path if path.is_absolute() else ROOT / path))
    try:
        relative = lexical.relative_to(temporary_root)
        resolved = lexical.resolve()
        resolved.relative_to(temporary_root)
    except ValueError as error:
        raise ValueError(
            "native bundle metadata output must remain below workspace .tmp"
        ) from error
    if not relative.parts:
        raise ValueError("native bundle metadata output cannot be workspace .tmp itself")
    current = temporary_root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"native bundle metadata output traverses a symlink: {current}")
    return resolved


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=native_bundle.BUNDLE_PATH)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    try:
        bundle = native_bundle.load_bundle(args.manifest.resolve(), ROOT)
        out = require_workspace_output(args.out)
        paths = write_metadata(bundle, out)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"native bundle metadata failed: {error}", file=sys.stderr)
        return 1
    print(
        "native_bundle_metadata_ok "
        f"bundle={bundle['bundle_id']} sbom={paths['sbom']} "
        f"licenses={paths['license_inventory']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
