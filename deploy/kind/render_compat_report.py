#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render a compact Markdown summary from collected Kind compatibility logs."""

from __future__ import annotations

import sys
from pathlib import Path


CHECKS = [
    ("QGIS read/render/filter/identify", "qgis-probe.log", ["valid True", "filter_names ['one']", "identify_names ['one']", "render_ok True"]),
    ("QGIS edit/save", "qgis-edit-probe.log", ["edit_ok True"]),
    ("GDAL/OGR load/read", "ogr-probe.log", ["feature_count 2", "loaded_rows", "load_feature_count 2"]),
    ("GeoServer WFS/WMS/WFS-T", "geoserver-probe.log", ["wfs_point_count", "wms_png_header 89504e470d0a1a0a", "wfst_transaction_ok True", "geoserver_probe_ok True"]),
    ("OSM PostGIS parity", "osm-postgis-parity.log", ["osm_postgis_to_quackgis_copy_ok True"]),
    ("Kind demo seed", "quackgis-demo.log", ["demo_ok True"]),
]


def status_for(path: Path, needles: list[str]) -> str:
    if not path.exists():
        return "not run"
    text = path.read_text(encoding="utf-8", errors="replace")
    if all(needle in text for needle in needles):
        return "pass"
    if "Error from server (NotFound)" in text or "not found" in text.lower():
        return "not run"
    return "fail"


def render(out_dir: Path) -> str:
    rows = []
    for label, log_name, needles in CHECKS:
        status = status_for(out_dir / log_name, needles)
        icon = {"pass": "✅", "fail": "❌", "not run": "➖"}[status]
        rows.append((label, f"{icon} {status}", log_name))

    passed = sum(1 for _, status, _ in rows if "pass" in status)
    failed = sum(1 for _, status, _ in rows if "fail" in status)
    body = [
        "# QuackGIS Kind compatibility report",
        "",
        f"Summary: **{passed} passed**, **{failed} failed**, **{len(rows) - passed - failed} not run**.",
        "",
        "| Check | Status | Log |",
        "|---|---|---|",
    ]
    body.extend(f"| {label} | {status} | `{log}` |" for label, status, log in rows)
    body.extend(
        [
            "",
            "Additional artifacts:",
            "",
            "- `kubernetes.txt` — namespace resources and pod/job state",
            "- `quackgis.log` — QuackGIS server log",
            "",
        ]
    )
    return "\n".join(body)


def main() -> int:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".tmp/compatibility")
    out_dir.mkdir(parents=True, exist_ok=True)
    report = render(out_dir)
    (out_dir / "README.md").write_text(report, encoding="utf-8")
    print(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
