-- SPDX-License-Identifier: Apache-2.0
-- Raster map algebra regression: st_pixeldata streams band pixels as (row, col,
-- value) rows; map algebra is then DuckDB-native SQL (WHERE/CASE/arithmetic).
-- The GDAL boundary stays narrow (one band read → DuckDB rows).
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/raster.sql
.bail off
.mode list

-- st_raster_info: the test raster is 4×3 Float64, one band
SELECT CASE WHEN width = 4 AND height = 3 THEN 'PASS' ELSE 'FAIL raster-info' END
FROM st_raster_info('tests/data/test_raster.asc');

-- st_raster_stats: min=1, max=12, mean=6.5, count=12
SELECT CASE WHEN min = 1.0 AND max = 12.0 AND abs(mean - 6.5) < 1e-9 AND count = 12
            THEN 'PASS' ELSE 'FAIL raster-stats' END
FROM st_raster_stats('tests/data/test_raster.asc', 1);

-- st_pixeldata: 12 pixels in a 4×3 grid
SELECT CASE WHEN count(*) = 12 THEN 'PASS' ELSE 'FAIL pixeldata-count' END
FROM st_pixeldata('tests/data/test_raster.asc', 1);

-- Map algebra via SQL: mean of pixels > 5 = 9.0 (6+7+8+9+10+11+12)/7
SELECT CASE WHEN abs(avg(value) - 9.0) < 1e-9 THEN 'PASS' ELSE 'FAIL mapalgebra-mean' END
FROM st_pixeldata('tests/data/test_raster.asc', 1) WHERE value > 5;

-- Classification: count pixels >= 8 = 5 (8,9,10,11,12)
SELECT CASE WHEN count(*) = 5 THEN 'PASS' ELSE 'FAIL mapalgebra-classify' END
FROM st_pixeldata('tests/data/test_raster.asc', 1) WHERE value >= 8;

-- Reclassification (CASE WHEN): values > 6 → 1, else 0; count of 1s = 6
SELECT CASE WHEN sum(CASE WHEN value > 6 THEN 1 ELSE 0 END) = 6
            THEN 'PASS' ELSE 'FAIL mapalgebra-reclass' END
FROM st_pixeldata('tests/data/test_raster.asc', 1);

-- Arithmetic: sum of value*2 = 2*(1+2+...+12) = 156
SELECT CASE WHEN sum(value * 2) = 156 THEN 'PASS' ELSE 'FAIL mapalgebra-arith' END
FROM st_pixeldata('tests/data/test_raster.asc', 1);

-- st_raster_transform: geotransform + computed bounds
-- Test raster is 4×3 at origin (0,3) with pixel size (1,-1) → bounds (0,0,4,3)
SELECT CASE WHEN xmin = 0.0 AND ymin = 0.0 AND xmax = 4.0 AND ymax = 3.0
                 AND origin_x = 0.0 AND origin_y = 3.0
                 AND pixel_w = 1.0 AND pixel_h = -1.0
            THEN 'PASS' ELSE 'FAIL raster-transform' END
FROM st_raster_transform('tests/data/test_raster.asc');

-- Pixel-to-geographic coordinate conversion using the transform
-- (col=2, row=1) → x = origin_x + col*pixel_w = 0 + 2*1 = 2
--                  y = origin_y + row*pixel_h = 3 + 1*(-1) = 2
SELECT CASE WHEN abs(
    (SELECT origin_x FROM st_raster_transform('tests/data/test_raster.asc')) +
    2 * (SELECT pixel_w FROM st_raster_transform('tests/data/test_raster.asc'))
    - 2.0) < 1e-9
            THEN 'PASS' ELSE 'FAIL pixel-to-geo' END;
