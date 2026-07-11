#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail closed if the DuckDB runtime image can install or omit pinned artifacts."""

from __future__ import annotations

import pathlib
import re
import sys


EXPECTED_COPIES = {
    "COPY quackgis-server /usr/local/bin/quackgis-server",
    "COPY duckdb /usr/local/bin/duckdb",
    "COPY libduckdb.so /opt/quackgis/lib/libduckdb.so",
    "COPY duckdb-home /opt/quackgis/duckdb",
    "COPY artifact-manifest.json /opt/quackgis/artifact-manifest.json",
}
FORBIDDEN = ("install ", "curl", "wget", "dnf", "apt-get", "apk ", "ADD ")


def validate(path: pathlib.Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    errors = []
    copies = {line for line in text.splitlines() if line.startswith("COPY ")}
    if copies != EXPECTED_COPIES:
        errors.append(f"{path}: runtime COPY set does not match pinned artifact contract")
    if not re.search(
        r"(?m)^FROM registry\.fedoraproject\.org/fedora-minimal@sha256:[0-9a-f]{64}$",
        text,
    ):
        errors.append(f"{path}: runtime base image must be pinned by sha256 digest")
    for expected in (
        "ENV HOME=/opt/quackgis/duckdb",
        "ENV QUACKGIS_DUCKDB_ADBC_DRIVER=/opt/quackgis/lib/libduckdb.so",
        "ENV LD_LIBRARY_PATH=/opt/quackgis/lib",
        "USER 999:999",
        'ENTRYPOINT ["/usr/local/bin/quackgis-server"]',
    ):
        if not re.search(rf"(?m)^{re.escape(expected)}\s*$", text):
            errors.append(f"{path}: missing {expected!r}")
    executable = "\n".join(
        line.lower() for line in text.splitlines() if not line.lstrip().startswith("#")
    )
    for token in FORBIDDEN:
        if token.lower() in executable:
            errors.append(f"{path}: forbidden online-install token {token!r}")
    return errors


def main(argv: list[str]) -> int:
    path = pathlib.Path(argv[1]) if len(argv) > 1 else pathlib.Path(
        "deploy/Containerfile.duckdb-runtime"
    )
    errors = validate(path)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"duckdb_runtime_static_check_ok path={path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
