#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate a copied multi-modal inventory evidence manifest.

This is a static promotion gate for ROADMAP 2.x asset work. It does not decode COG
or COPC/LAZ payloads; it ensures a packet cannot claim promoted real-inventory
evidence without naming the copied collections, object/row counts, URI policy,
CRS/epoch/checksum/footprint/lifecycle/restore/workload evidence, and the required
first COG + COPC/LAZ family coverage.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


VALID_CLAIMS = {"multimodal_inventory_wiring", "multimodal_inventory_promotion"}
VALID_STATUS = {"pass", "skip"}
VALID_FAMILIES = {
    "cog_raster",
    "copc_laz_pointcloud",
    "raster",
    "point_cloud",
    "three_d_tiles",
    "cad_bim",
    "imagery",
    "reality_capture",
}
COG_FAMILIES = {"cog_raster", "raster"}
COPC_FAMILIES = {"copc_laz_pointcloud", "point_cloud"}
SECRET_PATTERNS = (
    re.compile(r"(?i)(password|secret|token|signature|credential)=([^\s'\"]+)"),
    re.compile(r"(?i)X-Amz-(Signature|Credential|Security-Token)="),
)


class EvidenceError(ValueError):
    """The evidence packet is malformed or overclaims."""


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be an object")
    return value


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise EvidenceError(f"{label} must be a non-empty string")
    return value


def require_int(value: Any, label: str, *, positive: bool = False) -> int:
    if isinstance(value, bool):
        raise EvidenceError(f"{label} must be an integer")
    try:
        parsed = int(value)
    except (TypeError, ValueError) as error:
        raise EvidenceError(f"{label} must be an integer") from error
    if str(value) != str(parsed):
        raise EvidenceError(f"{label} must be an integer")
    if parsed < 0 or (positive and parsed == 0):
        qualifier = "positive" if positive else "non-negative"
        raise EvidenceError(f"{label} must be {qualifier}")
    return parsed


def assert_no_secret_uri(text: str, label: str) -> None:
    for pattern in SECRET_PATTERNS:
        if pattern.search(text):
            raise EvidenceError(f"{label} appears to contain a signed URL or secret-bearing URI")


def load_json(path: Path) -> dict[str, Any]:
    try:
        return require_object(json.loads(path.read_text(encoding="utf-8")), "manifest")
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"could not read manifest {path}: {error}") from error


def validate_collection(raw: Any, index: int) -> dict[str, Any]:
    collection = require_object(raw, f"collections[{index}]")
    family = require_string(collection.get("family"), f"collections[{index}].family")
    if family not in VALID_FAMILIES:
        raise EvidenceError(f"collections[{index}].family must be one of {sorted(VALID_FAMILIES)}")
    status = require_string(collection.get("status"), f"collections[{index}].status")
    if status not in VALID_STATUS:
        raise EvidenceError(f"collections[{index}].status must be one of {sorted(VALID_STATUS)}")
    name = require_string(collection.get("name"), f"collections[{index}].name")
    object_prefix = require_string(collection.get("object_prefix"), f"collections[{index}].object_prefix")
    assert_no_secret_uri(object_prefix, f"collections[{index}].object_prefix")
    row_count = require_int(collection.get("row_count"), f"collections[{index}].row_count", positive=True)
    object_count = require_int(
        collection.get("object_count"), f"collections[{index}].object_count", positive=True
    )
    object_bytes = require_int(
        collection.get("object_bytes"), f"collections[{index}].object_bytes", positive=True
    )
    if require_string(collection.get("uri_policy"), f"collections[{index}].uri_policy") != "non_secret_stable_uris":
        raise EvidenceError(f"collections[{index}].uri_policy must be non_secret_stable_uris")

    if status == "pass":
        for field in (
            "checksum_evidence",
            "crs_epoch_evidence",
            "footprint_evidence",
            "lifecycle_evidence",
            "restore_evidence",
            "workload_evidence",
        ):
            require_string(collection.get(field), f"collections[{index}].{field}")

    return {
        "family": family,
        "name": name,
        "status": status,
        "row_count": row_count,
        "object_count": object_count,
        "object_bytes": object_bytes,
    }


def validate_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    source_sha = require_string(manifest.get("source_sha"), "source_sha")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise EvidenceError("source_sha must be a 40-character lowercase Git SHA")
    claim = require_string(manifest.get("claim"), "claim")
    if claim not in VALID_CLAIMS:
        raise EvidenceError(f"claim must be one of {sorted(VALID_CLAIMS)}")
    require_string(manifest.get("storage_profile"), "storage_profile")
    require_string(manifest.get("vector_gate_evidence"), "vector_gate_evidence")

    raw_collections = manifest.get("collections")
    if not isinstance(raw_collections, list) or not raw_collections:
        raise EvidenceError("collections must be a non-empty list")
    collections = [validate_collection(raw, index) for index, raw in enumerate(raw_collections)]
    passed = [collection for collection in collections if collection["status"] == "pass"]
    skipped = [collection for collection in collections if collection["status"] == "skip"]
    if claim == "multimodal_inventory_promotion":
        if skipped:
            raise EvidenceError("multimodal_inventory_promotion requires every collection to pass")
        if not any(collection["family"] in COG_FAMILIES for collection in passed):
            raise EvidenceError("multimodal_inventory_promotion requires a passing COG/raster collection")
        if not any(collection["family"] in COPC_FAMILIES for collection in passed):
            raise EvidenceError("multimodal_inventory_promotion requires a passing COPC/LAZ point-cloud collection")
    return {
        "claim": claim,
        "source_sha": source_sha,
        "collections": collections,
        "passed": len(passed),
        "skipped": len(skipped),
        "rows": sum(collection["row_count"] for collection in collections),
        "objects": sum(collection["object_count"] for collection in collections),
        "object_bytes": sum(collection["object_bytes"] for collection in collections),
    }


def render(summary: dict[str, Any]) -> str:
    body = [
        "# Multi-modal inventory evidence check",
        "",
        f"Claim: `{summary['claim']}`",
        f"Source SHA: `{summary['source_sha']}`",
        f"Collections: {summary['passed']} passed, {summary['skipped']} skipped",
        f"Totals: rows={summary['rows']} objects={summary['objects']} object_bytes={summary['object_bytes']}",
        "",
        "| Collection | Family | Status | Rows | Objects | Bytes |",
        "|---|---|---|---:|---:|---:|",
    ]
    for collection in summary["collections"]:
        body.append(
            f"| {collection['name']} | `{collection['family']}` | {collection['status']} | "
            f"{collection['row_count']} | {collection['object_count']} | {collection['object_bytes']} |"
        )
    body.append("")
    return "\n".join(body)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        args.out.unlink(missing_ok=True)
        summary = validate_manifest(load_json(args.manifest))
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(render(summary), encoding="utf-8")
    except (EvidenceError, OSError) as error:
        print(f"multimodal inventory evidence check failed: {error}", file=sys.stderr)
        return 1
    print(f"multimodal_inventory_evidence_check_ok out={args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
