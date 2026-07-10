#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "deploy" / "kind" / "render_compat_report.py"
SPEC = importlib.util.spec_from_file_location("render_compat_report", MODULE_PATH)
assert SPEC and SPEC.loader
REPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPORT)


OSM_LOG = """
osm_extract_url https://download.geofabrik.de/europe/monaco-latest.osm.pbf
osm_extract_bytes 123456
postgis_osm_named_points_count 50
postgis_osm_named_lines_count 40
postgis_osm_named_multipolygons_count 30
postgis_osm_points_geojson_count 50
quackgis_osm_points_geojson_count 50
postgis_osm_lines_geojson_count 40
quackgis_osm_lines_geojson_count 40
postgis_osm_multipolygons_geojson_count 30
quackgis_osm_multipolygons_geojson_count 30
quackgis_osm_named_points_count 50
quackgis_osm_named_lines_count 40
quackgis_osm_named_multipolygons_count 30
osm_postgis_to_quackgis_copy_ok True
osm_mvt_points_tile_bytes 1000
osm_mvt_points_attribute_name Port Hercule
osm_mvt_points_attribute_ok True
osm_mvt_lines_tile_bytes 900
osm_mvt_lines_attribute_name Avenue
osm_mvt_lines_attribute_ok True
osm_mvt_multipolygons_tile_bytes 800
osm_mvt_multipolygons_attribute_name Monaco
osm_mvt_multipolygons_attribute_ok True
osm_mvt_ok True
qgis_osm_points_feature_count 50
qgis_osm_lines_feature_count 40
qgis_osm_multipolygons_feature_count 30
osm_qgis_open_ok True
osm_qgis_render_ok True
""".strip()


class RenderCompatReportTests(unittest.TestCase):
    def test_osm_real_data_metrics_include_counts_and_mvt_attributes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = Path(tmp)
            (out_dir / "osm-postgis-parity.log").write_text(OSM_LOG, encoding="utf-8")

            report = REPORT.metrics_report(out_dir)
            check = report["checks"]["osm_postgis_parity"]
            self.assertEqual(check["status"], "pass")
            metrics = check["metrics"]
            self.assertEqual(metrics["osm_extract_bytes"], 123456)
            self.assertEqual(metrics["postgis_points_named_count"], 50)
            self.assertEqual(metrics["quackgis_lines_geojson_count"], 40)
            self.assertEqual(metrics["qgis_multipolygons_feature_count"], 30)
            self.assertEqual(metrics["mvt_points_tile_bytes"], 1000)
            self.assertTrue(metrics["mvt_points_attribute_ok"])
            self.assertTrue(metrics["mvt_lines_attribute_ok"])
            self.assertTrue(metrics["mvt_multipolygons_attribute_ok"])

            markdown = REPORT.render(out_dir)
            self.assertIn("mvt_attributes=points,lines,multipolygons", markdown)


if __name__ == "__main__":
    unittest.main()
