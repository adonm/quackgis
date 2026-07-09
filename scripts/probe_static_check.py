#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Cheap static checks for Kind probe manifests.

This intentionally avoids requiring a live Kubernetes cluster. It is not a YAML
schema validator; it catches the common pre-Kind mistakes that otherwise waste a
full image build/probe cycle: tabs in manifests, empty manifest documents, and
missing top-level Kubernetes document markers.
"""

from __future__ import annotations

import pathlib
import re
import sys


def main(argv: list[str]) -> int:
    root = pathlib.Path(argv[1]) if len(argv) > 1 else pathlib.Path("deploy/kind")
    errors: list[str] = []
    manifests = sorted(root.glob("*.yaml"))
    for manifest in manifests:
        check_manifest(manifest, errors)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"probe_static_check_ok manifests={len(manifests)}")
    return 0


def check_manifest(manifest: pathlib.Path, errors: list[str]) -> None:
    text = manifest.read_text(encoding="utf-8")
    if "\t" in text:
        errors.append(f"{manifest}: tabs are not allowed in Kubernetes YAML")
    documents = [doc.strip() for doc in re.split(r"(?m)^---\s*(?:#.*)?$", text) if doc.strip()]
    if not documents:
        errors.append(f"{manifest}: no YAML documents found")
        return
    for index, document in enumerate(documents, start=1):
        keys = top_level_keys(document)
        required = {"apiVersion", "kind"}
        if keys.get("kind") != "Cluster":
            required.add("metadata")
        missing = required.difference(keys)
        if missing:
            missing_list = ", ".join(sorted(missing))
            errors.append(f"{manifest}: document {index} missing {missing_list}")


def top_level_keys(document: str) -> dict[str, str]:
    keys: dict[str, str] = {}
    for raw_line in document.splitlines():
        line = raw_line.rstrip()
        if not line or line.lstrip().startswith("#") or line.startswith(" "):
            continue
        key, separator, value = line.partition(":")
        if separator:
            keys[key] = value.strip()
    return keys


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
