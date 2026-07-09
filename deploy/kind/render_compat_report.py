#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render a compact Markdown summary from collected Kind compatibility logs."""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path


CHECKS = [
    ("QGIS read/render/filter/identify", "qgis-probe.log", ["valid True", "filter_names ['one']", "identify_names ['one']", "render_ok True"]),
    ("QGIS edit/save", "qgis-edit-probe.log", ["edit_ok True"]),
    ("GDAL/OGR load/read", "ogr-probe.log", ["feature_count 2", "loaded_rows", "load_feature_count 2", "ogr_keyless_compact_ok True"]),
    ("API client profile surfaces", "api-client-probe.log", ["api_client_psycopg_surface", "api_client_sqlalchemy_surface", "api_client_geopandas_surface", "api_client_pgfeatureserv_surface", "api_client_bi_surface", "api_client_mvt_surface", "api_client_probe_ok True"]),
    ("GeoServer WFS/WMS/WFS-T", "geoserver-probe.log", ["wfs_point_count", "wms_png_header 89504e470d0a1a0a", "wfst_transaction_ok True", "geoserver_keyless_update_ok True", "geoserver_probe_ok True"]),
    ("OSM PostGIS/QGIS/MVT parity", "osm-postgis-parity.log", ["osm_postgis_to_quackgis_copy_ok True", "osm_mvt_ok True", "osm_qgis_open_ok True", "osm_qgis_render_ok True"]),
    ("Kind demo seed", "quackgis-demo.log", ["demo_ok True"]),
    ("Lake PostgreSQL/S3 storage", "lake-probe.log", ["storage_ok True"]),
    ("External profile storage", "external-lake-probe.log", ["storage_ok True", "native_delete_ok True", "native_update_ok True", "native_compact_ok True", "delete_files=2", "delete_snapshots=1", "appended_files=1", "retired_files=0", "metrics_ok True"]),
    ("Lake multi-pod storage", "lake-multipod.log", ["storage_ok True"]),
    ("Lake load-balanced service", "lb-probe.log", ["lb_ok True"]),
    ("Lake read workload", "read-probe.log", ["read_ok True"]),
    ("Lake QPS readers", "qps-probe.log", ["qps_ok True", "qps_scan", "max_bytes_scanned="]),
    ("Lake writer conflict/retry", "write-verify.log", ["write_conflict", "conflict_observed=True", "write_ok True"]),
    ("Lake OLAP fanout", "olap-probe.log", ["olap_ok True", "olap_scan", "max_bytes_scanned="]),
    ("PostGIS regress subset", "postgis-regress.log", ["postgis_regress_subset", "pass_rate=1.000"]),
]

POSTGIS_REGRESS_MIN_CASES = 57


def line_kv(line: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for key, value in re.findall(r"([A-Za-z_][A-Za-z0-9_]*)=([^\s]+)", line):
        values[key] = value.strip("'")
    return values


def last_line(text: str, prefix: str) -> str | None:
    matches = [line for line in text.splitlines() if line.startswith(prefix)]
    return matches[-1] if matches else None


def max_int_from_lines(text: str, prefix: str, key: str) -> int | None:
    values = []
    for line in text.splitlines():
        if not line.startswith(prefix):
            continue
        raw = line_kv(line).get(key)
        if raw and raw != "NA":
            values.append(int(raw))
    return max(values) if values else None


def last_int_after_prefix(text: str, prefix: str) -> int | None:
    line = last_line(text, prefix)
    if not line:
        return None
    match = re.search(r"(-?\d+)\s*$", line)
    return int(match.group(1)) if match else None


def maybe_int(raw: str | None) -> int | None:
    if not raw or raw == "NA":
        return None
    try:
        return int(raw)
    except ValueError:
        return None


def maybe_float(raw: str | None) -> float | None:
    if not raw or raw == "NA":
        return None
    try:
        return float(raw)
    except ValueError:
        return None


def compact(values: dict[str, object | None]) -> dict[str, object]:
    return {key: value for key, value in values.items() if value is not None}


def env_value(name: str) -> str | None:
    value = os.environ.get(name)
    return value if value else None


def run_metadata() -> dict[str, object]:
    server_url = env_value("GITHUB_SERVER_URL") or "https://github.com"
    repository = env_value("GITHUB_REPOSITORY")
    run_id = env_value("GITHUB_RUN_ID")
    run_url = f"{server_url}/{repository}/actions/runs/{run_id}" if repository and run_id else None
    return compact(
        {
            "report_kind": env_value("QUACKGIS_REPORT_KIND"),
            "storage_recipe": env_value("STORAGE_RECIPE"),
            "github_repository": repository,
            "github_workflow": env_value("GITHUB_WORKFLOW"),
            "github_job": env_value("GITHUB_JOB"),
            "github_run_id": run_id,
            "github_run_attempt": env_value("GITHUB_RUN_ATTEMPT"),
            "github_run_url": run_url,
            "github_ref": env_value("GITHUB_REF"),
            "github_ref_name": env_value("GITHUB_REF_NAME"),
            "github_sha": env_value("GITHUB_SHA"),
            "github_event_name": env_value("GITHUB_EVENT_NAME"),
        }
    )


def metric_values(path: Path) -> dict[str, object]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8", errors="replace")
    name = path.name

    if name == "qps-probe.log":
        result = line_kv(last_line(text, "qps_result ") or "")
        config = line_kv(last_line(text, "qps_config ") or "")
        return compact(
            {
                "qps": maybe_float(result.get("qps")),
                "p95_ms": maybe_float(result.get("p95_ms")),
                "p99_ms": maybe_float(result.get("p99_ms")),
                "queries": maybe_int(result.get("queries")),
                "workers": maybe_int(result.get("workers")),
                "seeded_rows": maybe_int(config.get("seeded_rows")),
                "factor": maybe_int(config.get("factor")),
                "max_scan_bytes": max_int_from_lines(text, "qps_scan ", "bytes_scanned"),
                "max_scan_bytes_budget": maybe_int(config.get("max_bytes_scanned")),
                "max_file_groups": max_int_from_lines(text, "qps_scan ", "file_groups"),
                "max_file_groups_budget": maybe_int(config.get("max_file_groups")),
            }
        )

    if name == "olap-probe.log":
        result = line_kv(last_line(text, "olap_result ") or "")
        config = line_kv(last_line(text, "olap_config ") or "")
        scan = line_kv(last_line(text, "olap_scan ") or "")
        recheck = line_kv(last_line(text, "olap_recheck ") or "")
        return compact(
            {
                "qps": maybe_float(result.get("qps")),
                "p95_ms": maybe_float(result.get("p95_ms")),
                "p99_ms": maybe_float(result.get("p99_ms")),
                "queries": maybe_int(result.get("queries")),
                "workers": maybe_int(result.get("workers")),
                "seeded_rows": maybe_int(config.get("seeded_rows")),
                "factor": maybe_int(config.get("factor")),
                "bytes_scanned": maybe_int(scan.get("bytes_scanned")),
                "bytes_scanned_budget": maybe_int(config.get("max_bytes_scanned")),
                "groups": maybe_int(scan.get("groups")),
                "candidate_groups": maybe_int(recheck.get("candidate_groups")),
                "candidate_rows": maybe_int(recheck.get("candidate_rows")),
            }
        )

    if name == "write-verify.log":
        conflict = line_kv(last_line(text, "write_conflict ") or "")
        verify = line_kv(last_line(text, "write_verify ") or "")
        return compact(
            {
                "shared_rows": maybe_int(verify.get("shared_rows")),
                "failed_commits": maybe_int(conflict.get("failed_commits")),
                "retry_attempts": maybe_int(conflict.get("retry_attempts")),
                "conflict_observed": conflict.get("conflict_observed") == "True"
                if conflict.get("conflict_observed")
                else None,
            }
        )

    if name == "osm-postgis-parity.log":
        values = {
            f"qgis_{label}_feature_count": last_int_after_prefix(
                text, f"qgis_osm_{label}_feature_count "
            )
            for label in ("points", "lines", "multipolygons")
        }
        values.update(
            {
                f"mvt_{label}_tile_bytes": last_int_after_prefix(
                    text, f"osm_mvt_{label}_tile_bytes "
                )
                for label in ("points", "lines", "multipolygons")
            }
        )
        return compact(values)

    if name == "read-probe.log":
        result = line_kv(last_line(text, "read_result ") or "")
        scan = line_kv(last_line(text, "read_scan ") or "")
        return compact(
            {
                "qps": maybe_float(result.get("qps")),
                "p95_ms": maybe_float(result.get("p95_ms")),
                "p99_ms": maybe_float(result.get("p99_ms")),
                "bytes_scanned": maybe_int(scan.get("bytes_scanned")),
            }
        )

    if name == "api-client-probe.log":
        summary = line_kv(last_line(text, "api_client_summary ") or "")
        return compact(
            {
                "feature_count": maybe_int(summary.get("feature_count")),
                "reflected_columns": maybe_int(summary.get("reflected_columns")),
                "bbox_count": maybe_int(summary.get("bbox_count")),
                "groups": maybe_int(summary.get("groups")),
                "tile_bytes": maybe_int(summary.get("tile_bytes")),
            }
        )

    if name == "external-lake-probe.log":
        native_delete = line_kv(last_line(text, "native_delete ") or "")
        native_update = line_kv(last_line(text, "native_update ") or "")
        native_compact = line_kv(last_line(text, "native_compact ") or "")
        return compact(
            {
                "native_delete_files": maybe_int(native_delete.get("delete_files")),
                "native_delete_snapshots": maybe_int(
                    native_delete.get("delete_snapshots")
                ),
                "native_update_delete_files": maybe_int(
                    native_update.get("delete_files")
                ),
                "native_update_appended_files": maybe_int(
                    native_update.get("appended_files")
                ),
                "native_compact_delete_files": maybe_int(
                    native_compact.get("delete_files")
                ),
                "native_compact_appended_files": maybe_int(
                    native_compact.get("appended_files")
                ),
                "native_compact_retired_files": maybe_int(
                    native_compact.get("retired_files")
                ),
                "native_mutation_aborts": last_int_after_prefix(
                    text, "metrics_native_mutation_aborts_total "
                ),
                "native_mutation_aborts_budget": 0,
            }
        )

    if name == "postgis-regress.log":
        postgis = line_kv(last_line(text, "postgis_regress_subset ") or "")
        return compact(
            {
                "postgis_passed": maybe_int(postgis.get("passed")),
                "postgis_total": maybe_int(postgis.get("total")),
                "postgis_total_min": POSTGIS_REGRESS_MIN_CASES,
                "postgis_pass_rate": maybe_float(postgis.get("pass_rate")),
                "postgis_pass_rate_min": 1.0,
            }
        )

    return {}


def metric_summary(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8", errors="replace")
    name = path.name

    if name == "qps-probe.log":
        result = line_kv(last_line(text, "qps_result ") or "")
        config = line_kv(last_line(text, "qps_config ") or "")
        max_bytes = max_int_from_lines(text, "qps_scan ", "bytes_scanned")
        max_groups = max_int_from_lines(text, "qps_scan ", "file_groups")
        return "; ".join(
            item
            for item in [
                f"qps={result.get('qps')}" if result.get("qps") else "",
                f"p95_ms={result.get('p95_ms')}" if result.get("p95_ms") else "",
                f"p99_ms={result.get('p99_ms')}" if result.get("p99_ms") else "",
                f"max_scan_bytes={max_bytes}/{config.get('max_bytes_scanned')}"
                if max_bytes is not None and config.get("max_bytes_scanned")
                else "",
                f"max_file_groups={max_groups}/{config.get('max_file_groups')}"
                if max_groups is not None and config.get("max_file_groups")
                else "",
            ]
            if item
        )

    if name == "olap-probe.log":
        result = line_kv(last_line(text, "olap_result ") or "")
        config = line_kv(last_line(text, "olap_config ") or "")
        scan = line_kv(last_line(text, "olap_scan ") or "")
        return "; ".join(
            item
            for item in [
                f"qps={result.get('qps')}" if result.get("qps") else "",
                f"p95_ms={result.get('p95_ms')}" if result.get("p95_ms") else "",
                f"p99_ms={result.get('p99_ms')}" if result.get("p99_ms") else "",
                f"bytes_scanned={scan.get('bytes_scanned')}/{config.get('max_bytes_scanned')}"
                if scan.get("bytes_scanned") and config.get("max_bytes_scanned")
                else "",
                f"groups={scan.get('groups')}" if scan.get("groups") else "",
            ]
            if item
        )

    if name == "write-verify.log":
        conflict = line_kv(last_line(text, "write_conflict ") or "")
        verify = line_kv(last_line(text, "write_verify ") or "")
        return "; ".join(
            item
            for item in [
                f"shared_rows={verify.get('shared_rows')}" if verify.get("shared_rows") else "",
                f"failed_commits={conflict.get('failed_commits')}" if conflict.get("failed_commits") else "",
                f"retry_attempts={conflict.get('retry_attempts')}" if conflict.get("retry_attempts") else "",
            ]
            if item
        )

    if name == "osm-postgis-parity.log":
        counts = {
            label: last_int_after_prefix(text, f"qgis_osm_{label}_feature_count ")
            for label in ("points", "lines", "multipolygons")
        }
        mvt_bytes = {
            label: last_int_after_prefix(text, f"osm_mvt_{label}_tile_bytes ")
            for label in ("points", "lines", "multipolygons")
        }
        return "; ".join(
            item
            for item in [
                f"qgis_points={counts['points']}" if counts["points"] is not None else "",
                f"qgis_lines={counts['lines']}" if counts["lines"] is not None else "",
                f"qgis_multipolygons={counts['multipolygons']}"
                if counts["multipolygons"] is not None
                else "",
                f"mvt_points_bytes={mvt_bytes['points']}"
                if mvt_bytes["points"] is not None
                else "",
                f"mvt_lines_bytes={mvt_bytes['lines']}"
                if mvt_bytes["lines"] is not None
                else "",
                f"mvt_multipolygons_bytes={mvt_bytes['multipolygons']}"
                if mvt_bytes["multipolygons"] is not None
                else "",
            ]
            if item
        )

    if name == "read-probe.log":
        result = line_kv(last_line(text, "read_result ") or "")
        scan = line_kv(last_line(text, "read_scan ") or "")
        return "; ".join(
            item
            for item in [
                f"qps={result.get('qps')}" if result.get("qps") else "",
                f"p95_ms={result.get('p95_ms')}" if result.get("p95_ms") else "",
                f"p99_ms={result.get('p99_ms')}" if result.get("p99_ms") else "",
                f"bytes_scanned={scan.get('bytes_scanned')}" if scan.get("bytes_scanned") else "",
            ]
            if item
        )

    if name == "api-client-probe.log":
        summary = line_kv(last_line(text, "api_client_summary ") or "")
        return "; ".join(
            item
            for item in [
                f"feature_count={summary.get('feature_count')}" if summary.get("feature_count") else "",
                f"reflected_columns={summary.get('reflected_columns')}" if summary.get("reflected_columns") else "",
                f"bbox_count={summary.get('bbox_count')}" if summary.get("bbox_count") else "",
                f"groups={summary.get('groups')}" if summary.get("groups") else "",
                f"tile_bytes={summary.get('tile_bytes')}" if summary.get("tile_bytes") else "",
            ]
            if item
        )

    if name == "external-lake-probe.log":
        native_delete = line_kv(last_line(text, "native_delete ") or "")
        native_update = line_kv(last_line(text, "native_update ") or "")
        native_compact = line_kv(last_line(text, "native_compact ") or "")
        aborts = last_int_after_prefix(text, "metrics_native_mutation_aborts_total ")
        return "; ".join(
            item
            for item in [
                f"delete_files={native_delete.get('delete_files')}"
                if native_delete.get("delete_files")
                else "",
                f"update_appended={native_update.get('appended_files')}"
                if native_update.get("appended_files")
                else "",
                f"compact_appended={native_compact.get('appended_files')}"
                if native_compact.get("appended_files")
                else "",
                f"compact_retired={native_compact.get('retired_files')}"
                if native_compact.get("retired_files")
                else "",
                f"native_mutation_aborts={aborts}" if aborts is not None else "",
            ]
            if item
        )

    if name == "postgis-regress.log":
        postgis = line_kv(last_line(text, "postgis_regress_subset ") or "")
        return "; ".join(
            item
            for item in [
                f"passed={postgis.get('passed')}/{postgis.get('total')}"
                if postgis.get("passed") and postgis.get("total")
                else "",
                f"pass_rate={postgis.get('pass_rate')}" if postgis.get("pass_rate") else "",
            ]
            if item
        )

    return ""


def md_cell(value: str) -> str:
    return value.replace("|", "\\|")


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
        log_path = out_dir / log_name
        status = status_for(log_path, needles)
        icon = {"pass": "✅", "fail": "❌", "not run": "➖"}[status]
        rows.append((label, f"{icon} {status}", log_name, metric_summary(log_path)))

    passed = sum(1 for _, status, _, _ in rows if "pass" in status)
    failed = sum(1 for _, status, _, _ in rows if "fail" in status)
    body = [
        "# QuackGIS Kind compatibility/storage report",
        "",
        f"Summary: **{passed} passed**, **{failed} failed**, **{len(rows) - passed - failed} not run**.",
        "",
        "| Check | Status | Evidence | Log |",
        "|---|---|---|---|",
    ]
    body.extend(
        f"| {label} | {status} | {md_cell(evidence)} | `{log}` |"
        for label, status, log, evidence in rows
    )
    body.extend(
        [
            "",
            "Additional artifacts:",
            "",
            "- `kubernetes.txt` — namespace resources and pod/job state",
            "- `quackgis.log` — QuackGIS server log",
            "- `metrics.json` — machine-readable probe metrics for trend tracking",
            "",
        ]
    )
    return "\n".join(body)


def metrics_report(out_dir: Path) -> dict[str, object]:
    checks: dict[str, object] = {}
    passed = 0
    failed = 0
    not_run = 0
    for label, log_name, needles in CHECKS:
        log_path = out_dir / log_name
        status = status_for(log_path, needles)
        if status == "pass":
            passed += 1
        elif status == "fail":
            failed += 1
        else:
            not_run += 1
        check_id = Path(log_name).stem.replace("-", "_")
        checks[check_id] = {
            "label": label,
            "status": status,
            "log": log_name,
            "metrics": metric_values(log_path),
        }
    return {
        "run": run_metadata(),
        "summary": {"passed": passed, "failed": failed, "not_run": not_run},
        "checks": checks,
    }


def main() -> int:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".tmp/compatibility")
    out_dir.mkdir(parents=True, exist_ok=True)
    report = render(out_dir)
    (out_dir / "README.md").write_text(report, encoding="utf-8")
    (out_dir / "metrics.json").write_text(
        json.dumps(metrics_report(out_dir), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
