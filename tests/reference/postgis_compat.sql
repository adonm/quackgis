# SPDX-License-Identifier: Apache-2.0
-- Reference fixtures: known-good outputs for hard edge cases.
-- These exist so that regressions in the canonical backends (literal SedonaDB,
-- GEOS, GeographicLib, PROJ, GDAL) are caught immediately. Each block documents
-- the reference source (PostGIS docs, SedonaDB tests, or analytic ground truth).

.bail off
.mode list

-- =========================================================================
-- 1. Antipodal geodesic (Karney converges; Vincenty diverges in PostGIS).
--    Reference: GeographicLib direct calculation, WGS84.
--    London (51.5N, -0.13E) → Auckland (-36.85N, 174.76E).
--    Great-circle distance ≈ 18,335,000 m (hemisphere-spanning).
-- =========================================================================
SELECT CASE WHEN st_distancespheroid(
                st_point(-0.1278, 51.5074),
                st_point(174.7633, -36.8485))
            BETWEEN 18300000 AND 18400000
            THEN 'PASS' ELSE 'FAIL antipodal-spheroid' END;

-- =========================================================================
-- 2. London→Paris spheroid distance. Reference: PostGIS ST_DistanceSpheroid
--    = 343,924 m (well-documented). Our Karney impl matches.
-- =========================================================================
SELECT CASE WHEN abs(st_distancespheroid(
                st_point(-0.1278, 51.5074),
                st_point(2.3522, 48.8566)) - 343924.0) < 500.0
            THEN 'PASS' ELSE 'FAIL london-paris-spheroid' END;

-- =========================================================================
-- 3. CRS reprojection round-trip. EPSG:4326 → 3857 → 4326 should recover
--    the original point to within floating-point tolerance.
--    Reference: PROJ pipeline analytic identity.
-- =========================================================================
SELECT CASE WHEN abs(st_x(st_transform(
                st_transform(st_point(2.3522, 48.8566), 4326, 3857),
                3857, 4326)) - 2.3522) < 1e-6
            THEN 'PASS' ELSE 'FAIL crs-roundtrip' END;

-- =========================================================================
-- 4. Voronoi cocircular grid. 3×3 grid must yield exactly 9 cells.
--    Reference: PostGIS ST_VoronoiPolygons on the same input.
--    The earlier angle-sort prototype lost cells on this input.
-- =========================================================================
SELECT CASE WHEN (
    SELECT count(*) FROM st_dump(
        st_voronoipolygons(st_geomfromtext(
            'MULTIPOINT((0 0),(1 0),(2 0),(0 1),(1 1),(2 1),(0 2),(1 2),(2 2))'
        ))
    )) = 9 THEN 'PASS' ELSE 'FAIL voronoi-cocircular' END;

-- =========================================================================
-- 5. ST_Node on crossing lines. Two diagonal lines crossing at (2,2) must
--    produce ≥ 4 segments after noding. Reference: PostGIS ST_Node docs.
-- =========================================================================
SELECT CASE WHEN st_numpoints(st_node(st_geomfromtext(
            'MULTILINESTRING((0 0,4 4),(0 4,4 0))'))) >= 8
            THEN 'PASS' ELSE 'FAIL node-crossing' END;

-- =========================================================================
-- 6. ST_Polygonize from a closed ring. A single closed ring must produce
--    exactly one polygon. Reference: PostGIS ST_Polygonize.
-- =========================================================================
SELECT CASE WHEN st_numgeometries(st_polygonize(st_geomfromtext(
            'LINESTRING(0 0,4 0,4 4,0 4,0 0)'))) = 1
            THEN 'PASS' ELSE 'FAIL polygonize-ring' END;

-- =========================================================================
-- 7. ST_BuildArea with a hole. Exterior 4×4 minus interior 1×1 = area 15.
--    Reference: PostGIS ST_BuildArea; analytic ground truth.
-- =========================================================================
SELECT CASE WHEN abs(st_area(st_buildarea(st_geomfromtext(
            'MULTILINESTRING((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'
        ))) - 15.0) < 1e-9
            THEN 'PASS' ELSE 'FAIL buildarea-hole' END;

-- =========================================================================
-- 8. Empty geometry handling. ST_IsEmpty on GEOMETRYCOLLECTION EMPTY = true.
--    Known delta: ST_Dimension returns 0 (matching SedonaDB) rather than
--    PostGIS's -1. Both paths agree here; documented in ROADMAP.
-- =========================================================================
SELECT CASE WHEN st_isempty(st_geomfromtext('GEOMETRYCOLLECTION EMPTY'))
            THEN 'PASS' ELSE 'FAIL empty-iscollection' END;

-- =========================================================================
-- 9. NULL propagation. Any ST_* on NULL must return NULL, not crash.
--    Reference: PostGIS NULL semantics; our fail-closed policy.
-- =========================================================================
SELECT CASE WHEN st_area(CAST(NULL AS BLOB)) IS NULL
            THEN 'PASS' ELSE 'FAIL null-propagation' END;

-- =========================================================================
-- 10. Raster pixel streaming + map algebra. The 4×3 test raster has values
--     1..12. Sum = 78, mean = 6.5, count = 12.
--     Reference: analytic ground truth.
-- =========================================================================
SELECT CASE WHEN abs((SELECT sum(value) FROM st_pixeldata('tests/data/test_raster.asc', 1)) - 78.0) < 1e-9
            THEN 'PASS' ELSE 'FAIL raster-sum' END;

SELECT CASE WHEN abs((SELECT avg(value) FROM st_pixeldata('tests/data/test_raster.asc', 1)) - 6.5) < 1e-9
            THEN 'PASS' ELSE 'FAIL raster-mean' END;
