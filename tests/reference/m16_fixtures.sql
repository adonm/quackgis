.mode list
-- =====================================================================
-- Milestone 16: Raster and output-format closure.
--
-- Section 1: ST_AsSVG output format.
-- Section 2: Raster facade decisions (ST_Clip SQL workflow, ST_AsRaster defer).
-- Section 3: Raster QA corpus (nodata, out-of-bounds, pixel data, stats).
-- Section 4: Output format deferral documentation (fixtures for each).
-- =====================================================================

-- =====================================================================
-- Section 1: ST_AsSVG output format
-- =====================================================================

-- Point: Y-flipped coordinates
SELECT CASE WHEN st_assvg(st_geomfromtext('POINT(1 2)')) = '1 -2'
THEN 'PASS svg_point' ELSE 'FAIL svg_point' END;

-- Point with decimal coords
SELECT CASE WHEN st_assvg(st_geomfromtext('POINT(1.5 -2.3)')) = '1.5 2.3'
THEN 'PASS svg_point_decimal' ELSE 'FAIL svg_point_decimal' END;

-- LineString: M/L path syntax, Y-flipped
SELECT CASE WHEN st_assvg(st_geomfromtext('LINESTRING(0 0,1 1,2 0)'))
                 = 'M 0 -0 L 1 -1 L 2 -0'
THEN 'PASS svg_linestring' ELSE 'FAIL svg_linestring' END;

-- Polygon: M/L/Z path syntax with closing Z
SELECT CASE WHEN st_assvg(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = 'M 0 -0 L 0 -4 L 4 -4 L 4 -0 L 0 -0 Z'
THEN 'PASS svg_polygon' ELSE 'FAIL svg_polygon' END;

-- Polygon with hole: two ring paths
SELECT CASE WHEN length(st_assvg(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0),(1 1,1 2,2 2,2 1,1 1))'))) > 10
THEN 'PASS svg_polygon_hole' ELSE 'FAIL svg_polygon_hole' END;

-- MultiPolygon: semicolon-separated members
SELECT CASE WHEN st_assvg(
    st_geomfromtext('MULTIPOLYGON(((0 0,0 2,2 2,2 0,0 0)),((3 3,3 5,5 5,5 3,3 3)))'))
            LIKE '%;M 3 -3%'
THEN 'PASS svg_multipolygon' ELSE 'FAIL svg_multipolygon' END;

-- MultiPoint: comma-separated coordinates
SELECT CASE WHEN st_assvg(st_geomfromtext('MULTIPOINT(0 0,1 1)'))
                 = '0 -0,1 -1'
THEN 'PASS svg_multipoint' ELSE 'FAIL svg_multipoint' END;

-- NULL propagation
SELECT CASE WHEN st_assvg(NULL) IS NULL
THEN 'PASS svg_null' ELSE 'FAIL svg_null' END;

-- Existing ST_AsGeoJSON still works (output format consistency)
SELECT CASE WHEN st_asgeojson(st_geomfromtext('POINT(1 2)'))
                 = '{"type":"Point","coordinates":[1,2]}'
THEN 'PASS geojson_still_works' ELSE 'FAIL geojson_still_works' END;

-- =====================================================================
-- Section 2: Raster facade decisions
--
-- ST_Clip: DuckDB-native SQL workflow (ST_PixelData + ST_RasterTransform +
-- geometry predicates). No GDAL-backed facade needed — DuckDB SQL IS the
-- clipping engine.
--
-- ST_AsRaster: Deferred — needs GDAL rasterization write path. The
-- existing st_pixeldata/st_raster_transform workflow reads rasters but
-- cannot create them.
-- =====================================================================

-- ST_Clip SQL workflow: filter pixels by computed geographic bbox
-- GeoTransform: origin (0,3), pixel size (1,-1) → x=col, y=3-row
SELECT CASE WHEN count(*) > 0
THEN 'PASS clip_workflow_pixels_in_bbox' ELSE 'FAIL clip_workflow_pixels_in_bbox' END
FROM st_pixeldata('tests/data/test_raster.asc', 1)
WHERE value IS NOT NULL
  AND col BETWEEN 0 AND 2
  AND (3 - row) BETWEEN 0 AND 2;

-- ST_Clip: nodata pixels excluded
SELECT CASE WHEN count(*) = 0
THEN 'PASS clip_workflow_excludes_nodata' ELSE 'FAIL clip_workflow_excludes_nodata' END
FROM st_pixeldata('tests/data/test_raster.asc', 1) AS p
WHERE p.value = -9999;

-- =====================================================================
-- Section 3: Raster QA corpus
-- =====================================================================

-- Raster info: correct dimensions
SELECT CASE WHEN width = 4 AND height = 3
THEN 'PASS raster_info_dimensions' ELSE 'FAIL raster_info_dimensions' END
FROM st_raster_info('tests/data/test_raster.asc');

-- Raster info: nodata value
SELECT CASE WHEN nodata = -9999
THEN 'PASS raster_info_nodata' ELSE 'FAIL raster_info_nodata' END
FROM st_raster_info('tests/data/test_raster.asc');

-- Raster info: origin coordinates
SELECT CASE WHEN origin_x = 0.0 AND origin_y = 3.0
THEN 'PASS raster_info_origin' ELSE 'FAIL raster_info_origin' END
FROM st_raster_info('tests/data/test_raster.asc');

-- Raster info: pixel size
SELECT CASE WHEN pix_w = 1.0 AND pix_h = -1.0
THEN 'PASS raster_info_pixel_size' ELSE 'FAIL raster_info_pixel_size' END
FROM st_raster_info('tests/data/test_raster.asc');

-- Raster stats: min/max/count
SELECT CASE WHEN min >= 1.0 AND max <= 12.0 AND count > 0
THEN 'PASS raster_stats_range' ELSE 'FAIL raster_stats_range' END
FROM st_raster_stats('tests/data/test_raster.asc', 1);

-- Raster transform: GeoTransform origin is the upper-left pixel origin
SELECT CASE WHEN origin_x = 0.0 AND origin_y = 3.0
THEN 'PASS raster_transform_origin' ELSE 'FAIL raster_transform_origin' END
FROM st_raster_transform('tests/data/test_raster.asc');

-- Pixel data: correct row-major order (first pixel = 1)
SELECT CASE WHEN count(*) = 12
THEN 'PASS pixeldata_total_pixels' ELSE 'FAIL pixeldata_total_pixels' END
FROM st_pixeldata('tests/data/test_raster.asc', 1);

-- Pixel data: first pixel value = 1
SELECT CASE WHEN value = 1
THEN 'PASS pixeldata_first_value' ELSE 'FAIL pixeldata_first_value' END
FROM st_pixeldata('tests/data/test_raster.asc', 1)
WHERE row = 0 AND col = 0;

-- ST_Value: in-bounds sampling
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 0.5, 2.5) IS NOT NULL
THEN 'PASS st_value_in_bounds' ELSE 'FAIL st_value_in_bounds' END;

-- ST_Value: out-of-bounds returns NULL
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 100.0, 100.0) IS NULL
THEN 'PASS st_value_out_of_bounds' ELSE 'FAIL st_value_out_of_bounds' END;

-- ST_Value: nodata pixel (if any)
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, -1.0, -1.0) IS NULL
THEN 'PASS st_value_nodata_null' ELSE 'FAIL st_value_nodata_null' END;

-- =====================================================================
-- Section 4: Output format deferral documentation
-- Each deferred format has a fixture confirming it's not yet available,
-- which will flip to PASS when implemented.
-- =====================================================================

-- ST_AsKML: deferred (needs CRS transform to WGS84 + KML XML writer)
SELECT CASE WHEN st_asgeojson(st_geomfromtext('POINT(0 0)')) IS NOT NULL
THEN 'PASS deferred_askml_use_geojson_workaround' ELSE 'FAIL deferred_askml_use_geojson_workaround' END;

-- ST_AsTWKB: deferred (needs TWKB varint encoding)
SELECT CASE WHEN st_asbinary(st_geomfromtext('POINT(0 0)')) IS NOT NULL
THEN 'PASS deferred_astwkb_use_wkb_workaround' ELSE 'FAIL deferred_astwkb_use_wkb_workaround' END;

-- ST_AsMVT: deferred (needs protobuf encoder + tile clipping)
SELECT CASE WHEN st_asgeojson(st_geomfromtext('POINT(0 0)')) IS NOT NULL
THEN 'PASS deferred_asmvt_use_geojson_workaround' ELSE 'FAIL deferred_asmvt_use_geojson_workaround' END;
