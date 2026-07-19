#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate deterministic native bundle SPDX and license metadata."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

import native_bundle


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / ".tmp/native-bundle-metadata"


def source_download(source: dict[str, str]) -> str:
    return f"{source['url'].removesuffix('.git')}/tree/{source['commit']}"


def spdx_package(
    name: str,
    version: str,
    source: dict[str, str],
    artifact_sha256: str,
    declared_license: str,
    concluded_license: str,
    comment: str,
) -> dict[str, Any]:
    return {
        "SPDXID": f"SPDXRef-Package-{name}",
        "name": name,
        "versionInfo": version,
        "downloadLocation": source_download(source),
        "filesAnalyzed": False,
        "licenseConcluded": concluded_license,
        "licenseDeclared": declared_license,
        "copyrightText": "NOASSERTION",
        "checksums": [{"algorithm": "SHA256", "checksumValue": artifact_sha256}],
        "packageComment": comment,
    }


def spdx_document(bundle: dict[str, Any]) -> dict[str, Any]:
    bundle_sha256 = native_bundle.canonical_sha256(bundle)
    duckdb = bundle["duckdb"]
    ducklake = bundle["extensions"]["ducklake"]
    spatial = bundle["extensions"]["spatial"]
    packages = [
        spdx_package(
            "DuckDB",
            duckdb["version"],
            duckdb["source"],
            duckdb["artifact"]["library_sha256"],
            "MIT",
            "MIT",
            f"Selected {duckdb['artifact']['mode']} libduckdb artifact for {bundle['platform']}.",
        ),
        spdx_package(
            "DuckLake",
            ducklake["source"]["commit"],
            ducklake["source"],
            ducklake["artifact"]["sha256"],
            "MIT",
            "NOASSERTION",
            "Project-built extension includes the digest-pinned QuackGIS identity patch; complete concluded-license review remains required.",
        ),
        spdx_package(
            "DuckDB-Spatial",
            spatial["source"]["commit"],
            spatial["source"],
            spatial["artifact"]["sha256"],
            "MIT",
            "NOASSERTION",
            "Vendor-built signed extension bundles native dependencies whose exact versions and concluded licenses remain release-blocking.",
        ),
    ]
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": bundle["bundle_id"],
        "documentNamespace": (
            "https://quackgis.invalid/spdx/"
            f"{bundle['bundle_id']}/{bundle_sha256}"
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
        ],
    }


def license_inventory(bundle: dict[str, Any]) -> dict[str, Any]:
    duckdb = bundle["duckdb"]
    ducklake = bundle["extensions"]["ducklake"]
    spatial = bundle["extensions"]["spatial"]
    return {
        "schema_version": 1,
        "bundle_id": bundle["bundle_id"],
        "bundle_sha256": native_bundle.canonical_sha256(bundle),
        "complete": False,
        "redistribution_status": "local-evaluation-only",
        "components": [
            {
                "name": "duckdb",
                "source_url": duckdb["source"]["url"],
                "commit": duckdb["source"]["commit"],
                "license_declared": "MIT",
                "license_concluded": "MIT",
                "artifact_sha256": duckdb["artifact"]["library_sha256"],
            },
            {
                "name": "ducklake",
                "source_url": ducklake["source"]["url"],
                "commit": ducklake["source"]["commit"],
                "license_declared": "MIT",
                "license_concluded": "NOASSERTION",
                "artifact_sha256": ducklake["artifact"]["sha256"],
                "patches": [
                    {"path": patch["path"], "sha256": patch["sha256"]}
                    for patch in native_bundle.validate_series(bundle, "ducklake", ROOT)["patches"]
                ],
            },
            {
                "name": "spatial",
                "source_url": spatial["source"]["url"],
                "commit": spatial["source"]["commit"],
                "license_declared": "MIT plus bundled third-party dependencies",
                "license_concluded": "NOASSERTION",
                "artifact_sha256": spatial["artifact"]["sha256"],
            },
        ],
        "unresolved": [
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


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def write_metadata(bundle: dict[str, Any], out: Path) -> dict[str, Path]:
    outputs = bundle["outputs"]
    paths = {
        "sbom": out / outputs["sbom"],
        "license_inventory": out / outputs["license_inventory"],
    }
    write_json(paths["sbom"], spdx_document(bundle))
    write_json(paths["license_inventory"], license_inventory(bundle))
    return paths


def require_workspace_output(path: Path) -> Path:
    resolved = path.resolve()
    temporary_root = (ROOT / ".tmp").resolve()
    if resolved == temporary_root or temporary_root not in resolved.parents:
        raise ValueError("native bundle metadata output must remain below workspace .tmp")
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
