#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Static guard for QuackGIS runtime image posture.

The maintained runtime image should stay boring: one prebuilt Rust binary copied
into a slim base, non-root user, no native GIS packages, and no build toolchain in
the runtime stage.
"""

from __future__ import annotations

import pathlib
import re
import sys


FORBIDDEN_RUNTIME_TOKENS = (
    "apt-get",
    "dnf ",
    "microdnf",
    "yum ",
    "apk ",
    "gdal",
    "geos",
    "proj-bin",
    "libproj",
    "postgis",
    "postgresql-server",
    "duckdb",
    "cargo ",
    "rustc",
)


def main(argv: list[str]) -> int:
    path = pathlib.Path(argv[1]) if len(argv) > 1 else pathlib.Path("deploy/Containerfile.runtime")
    text = path.read_text(encoding="utf-8")
    errors: list[str] = []

    copy_lines = [line for line in text.splitlines() if line.startswith("COPY ")]
    if copy_lines != ["COPY quackgis-server /usr/local/bin/quackgis-server"]:
        errors.append(f"{path}: runtime image must copy only quackgis-server")

    if not re.search(r"(?m)^USER\s+999:999\s*$", text):
        errors.append(f"{path}: runtime image must run as non-root USER 999:999")

    if not re.search(r'(?m)^ENTRYPOINT \["/usr/local/bin/quackgis-server"\]\s*$', text):
        errors.append(f"{path}: runtime image must enter through quackgis-server")

    lowered = "\n".join(
        line for line in text.lower().splitlines() if not line.lstrip().startswith("#")
    )
    for token in FORBIDDEN_RUNTIME_TOKENS:
        if token in lowered:
            errors.append(f"{path}: forbidden runtime token {token!r}")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    print(f"runtime_static_check_ok path={path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
