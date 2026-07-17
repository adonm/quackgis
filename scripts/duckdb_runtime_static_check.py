#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail closed if the DuckDB runtime image can install or omit pinned artifacts."""

from __future__ import annotations

import json
import pathlib
import re
import sys


EXPECTED_COPIES = {
    "COPY quackgis-server /usr/local/bin/quackgis-server",
    "COPY quackgis-migrate /usr/local/bin/quackgis-migrate",
    "COPY quackgis-rest /usr/local/bin/quackgis-rest",
    "COPY quackgis-bootstrap /usr/local/bin/quackgis-bootstrap",
    "COPY quackgis-worker-edge /usr/local/bin/quackgis-worker-edge",
    "COPY quackgis-client /usr/local/bin/quackgis-client",
    "COPY quackgis-keygen /usr/local/bin/quackgis-keygen",
    "COPY duckdb /usr/local/bin/duckdb",
    "COPY libduckdb.so /opt/quackgis/lib/libduckdb.so",
    "COPY duckdb-home /opt/quackgis/duckdb",
    "COPY artifact-manifest.json /opt/quackgis/artifact-manifest.json",
    "COPY licenses /opt/quackgis/licenses",
}
ROOT = pathlib.Path(__file__).resolve().parent.parent
PINNED_DUCKLAKE_SHA256 = json.loads(
    (ROOT / "patches/ducklake/pin.json").read_text(encoding="utf-8")
)["artifact_sha256"]
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
        "ENV QUACKGIS_DUCKLAKE_EXTENSION=/opt/quackgis/duckdb/.duckdb/extensions/v1.5.4/linux_amd64/ducklake.duckdb_extension",
        f"ENV QUACKGIS_DUCKLAKE_EXTENSION_SHA256={PINNED_DUCKLAKE_SHA256}",
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
