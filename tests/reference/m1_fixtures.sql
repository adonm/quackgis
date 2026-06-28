-- SPDX-License-Identifier: Apache-2.0
-- Month 1 reference fixtures: expanded edge-case coverage for PostGIS/SedonaDB
-- compatibility. Each section covers a hard semantic area.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m1_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. Invalid geometry handling (bowtie polygon — self-intersecting)
-- ======================================================================
-- ST_MakeValid should produce valid geometry from a bowtie.
SELECT CASE WHEN st_isvalid(st_makevalid(st_geomfromtext(
            'POLYGON((0 0,4 0,0 4,4 4,0 0))'  -- bowtie
        ))) = true
            THEN 'PASS makevalid bowtie' ELSE 'FAIL makevalid bowtie' END;

-- ST_IsValid on bowtie should be false.
SELECT CASE WHEN st_isvalid(st_geomfromtext(
            'POLYGON((0 0,4 0,0 4,4 4,0 0))'
        )) = false
            THEN 'PASS isvalid bowtie false' ELSE 'FAIL isvalid bowtie' END;

-- ======================================================================
-- 2. Empty geometry semantics
-- ======================================================================
SELECT CASE WHEN st_isempty(st_geomfromtext('POINT EMPTY')) = true
            THEN 'PASS empty point' ELSE 'FAIL empty point' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('GEOMETRYCOLLECTION EMPTY')) = true
            THEN 'PASS empty collection' ELSE 'FAIL empty collection' END;
SELECT CASE WHEN st_area(st_geomfromtext('POLYGON EMPTY')) = 0.0
            THEN 'PASS empty polygon area' ELSE 'FAIL empty polygon area' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('LINESTRING EMPTY')) = true
            THEN 'PASS empty line' ELSE 'FAIL empty line' END;

-- ======================================================================
-- 3. NULL propagation
-- ======================================================================
SELECT CASE WHEN st_area(NULL) IS NULL
            THEN 'PASS null area' ELSE 'FAIL null area' END;
SELECT CASE WHEN st_distance(NULL, st_geomfromtext('POINT(0 0)')) IS NULL
            THEN 'PASS null distance' ELSE 'FAIL null distance' END;
SELECT CASE WHEN st_intersects(NULL, NULL) IS NULL
            THEN 'PASS null intersects' ELSE 'FAIL null intersects' END;
SELECT CASE WHEN st_astext(CAST(NULL AS BLOB)) IS NULL
            THEN 'PASS null astext' ELSE 'FAIL null astext' END;

-- ======================================================================
-- 4. Antimeridian geography (lon ±180)
-- ======================================================================
-- Distance between points near the antimeridian should be small, not ~earth circumference.
SELECT CASE WHEN st_distancesphere(st_point(179.99, 0), st_point(-179.99, 0)) < 50000.0
            THEN 'PASS antimeridian distance' ELSE 'FAIL antimeridian distance' END;

-- Spheroid variant
SELECT CASE WHEN st_distancespheroid(st_point(179.99, 0), st_point(-179.99, 0)) < 50000.0
            THEN 'PASS antimeridian spheroid' ELSE 'FAIL antimeridian spheroid' END;

-- ======================================================================
-- 5. CRS round-trip (PROJ)
-- ======================================================================
-- EPSG:4326 → EPSG:3857 → EPSG:4326 should round-trip within 1e-6 degrees.
WITH p AS (SELECT st_geomfromtext('POINT(-0.1278 51.5074)') AS g)
SELECT CASE WHEN abs(st_x(st_transform(st_transform(g, 4326, 3857), 3857, 4326))
                     - st_x(g)) < 1e-6
              AND abs(st_y(st_transform(st_transform(g, 4326, 3857), 3857, 4326))
                     - st_y(g)) < 1e-6
            THEN 'PASS CRS round-trip' ELSE 'FAIL CRS round-trip' END FROM p;

-- ======================================================================
-- 6. GEOS Snap degeneracy (zero tolerance = identity)
-- ======================================================================
WITH ab AS (
    SELECT st_geomfromtext('LINESTRING(0 0,1 1)') AS a,
           st_geomfromtext('LINESTRING(0 0,1 1)') AS b
)
SELECT CASE WHEN st_astext(st_snap(a, b, 0.0)) = st_astext(a)
            THEN 'PASS snap zero tolerance' ELSE 'FAIL snap zero tolerance' END FROM ab;

-- ======================================================================
-- 7. Voronoi degeneracy (single point → empty/geometry)
-- ======================================================================
SELECT CASE WHEN st_geometrytype(st_voronoipolygons(st_geomfromtext('MULTIPOINT((5 5))')))
                 IN ('ST_GeometryCollection', 'ST_MultiPolygon')
            THEN 'PASS voronoi single point' ELSE 'FAIL voronoi single point' END;

-- ======================================================================
-- 8. Force-dimension family (Month 1 namespace addition)
-- ======================================================================
-- st_force3d adds Z dimension; sedona_st_force3d does the same via literal kernel.
SELECT CASE WHEN sedona_st_hasz(sedona_st_force3d(st_geomfromtext('POINT(1 2)'), 0.0)) = true
            THEN 'PASS force3d literal' ELSE 'FAIL force3d literal' END;
-- st_force3dz is an alias of st_force3d.
SELECT CASE WHEN sedona_st_hasz(st_force3dz(st_geomfromtext('POINT(1 2)'), 0.0)) = true
            THEN 'PASS force3dz alias' ELSE 'FAIL force3dz alias' END;
-- st_force3dm adds M dimension.
SELECT CASE WHEN sedona_st_hasm(st_force3dm(st_geomfromtext('POINT(1 2)'), 0.0)) = true
            THEN 'PASS force3dm' ELSE 'FAIL force3dm' END;
-- st_force4d adds both Z and M.
SELECT CASE WHEN sedona_st_hasz(st_force4d(st_geomfromtext('POINT(1 2)'), 0.0, 0.0)) = true
             AND sedona_st_hasm(st_force4d(st_geomfromtext('POINT(1 2)'), 0.0, 0.0)) = true
            THEN 'PASS force4d' ELSE 'FAIL force4d' END;

-- Delta retirement: 1-arg overloads match PostGIS defaults (z=0, m=0)
SELECT CASE WHEN sedona_st_zmflag(st_force3d(st_geomfromtext('POINT(1 2)'))) = 2
            THEN 'PASS force3d_1arg_default' ELSE 'FAIL force3d_1arg_default' END;
SELECT CASE WHEN sedona_st_zmflag(st_force3dm(st_geomfromtext('POINT(1 2)'))) = 1
            THEN 'PASS force3dm_1arg_default' ELSE 'FAIL force3dm_1arg_default' END;
SELECT CASE WHEN sedona_st_zmflag(st_force4d(st_geomfromtext('POINT(1 2)'))) = 3
            THEN 'PASS force4d_1arg_default' ELSE 'FAIL force4d_1arg_default' END;

-- Delta retirement: ST_Dimension(EMPTY) returns -1 (PostGIS parity)
SELECT CASE WHEN st_dimension(st_geomfromtext('POINT EMPTY')) = -1
            THEN 'PASS dimension_empty_negative1' ELSE 'FAIL dimension_empty_negative1' END;

-- ======================================================================
-- 9. Large coordinate values (round-trip stability)
-- ======================================================================
SELECT CASE WHEN st_x(st_geomfromtext('POINT(123456789.123456 987654321.654321)'))
                 = 123456789.123456
            THEN 'PASS large coords' ELSE 'FAIL large coords' END;

-- ======================================================================
-- 10. Collection nesting
-- ======================================================================
SELECT CASE WHEN st_numgeometries(st_geomfromtext(
            'GEOMETRYCOLLECTION(GEOMETRYCOLLECTION(POINT(1 1),POINT(2 2)),POINT(3 3))'
        )) = 2  -- top-level count
            THEN 'PASS nested collection count' ELSE 'FAIL nested collection count' END;

-- ======================================================================
-- 11. Polygon with multiple holes
-- ======================================================================
SELECT CASE WHEN st_numinteriorrings(st_geomfromtext(
            'POLYGON((0 0,10 0,10 10,0 10,0 0),(1 1,2 1,2 2,1 2,1 1),(5 5,6 5,6 6,5 6,5 5))'
        )) = 2
            THEN 'PASS two holes' ELSE 'FAIL two holes' END;

-- ======================================================================
-- 12. Degenerate line (single segment, zero length)
-- ======================================================================
SELECT CASE WHEN st_length(st_geomfromtext('LINESTRING(5 5,5 5)')) = 0.0
            THEN 'PASS degenerate line' ELSE 'FAIL degenerate line' END;
