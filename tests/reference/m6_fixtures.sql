-- SPDX-License-Identifier: Apache-2.0
-- Month 6 reference fixtures: workflow verification + PostGIS migration examples.
-- These test the canonical user workflows end-to-end, proving that the most
-- common PostGIS SQL patterns work correctly in this extension.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m6_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. GeoParquet-style ingest + scan
-- ======================================================================
-- Simulate a GeoParquet table: geometry as WKB BLOB.

CREATE TABLE points AS
    SELECT i AS id, st_point(i * 0.1, i * 0.2) AS geom
    FROM range(0, 100) t(i);

SELECT CASE WHEN (SELECT count(*) FROM points) = 100
            THEN 'PASS ingest count' ELSE 'FAIL ingest count' END;

SELECT CASE WHEN (SELECT st_geometrytype(geom) FROM points LIMIT 1) = 'ST_Point'
            THEN 'PASS ingest type' ELSE 'FAIL ingest type' END
FROM points;

-- ======================================================================
-- 2. CRS transform join
-- ======================================================================

SELECT CASE WHEN abs(st_x(st_transform(st_geomfromtext('POINT(-0.1278 51.5074)'),
                                        4326, 3857)) - (-14227.16)) < 1.0
            THEN 'PASS crs transform webmercator' ELSE 'FAIL crs transform webmercator' END;

SELECT CASE WHEN abs(st_x(st_transform(st_geomfromtext('POINT(0 0)'), 4326, 3857))) < 1.0
            THEN 'PASS crs transform origin' ELSE 'FAIL crs transform origin' END;

-- ======================================================================
-- 3. Bbox prefilter + exact predicate join
-- ======================================================================

WITH a(id, geom) AS (
    SELECT * FROM (VALUES
        (1, st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')),
        (2, st_geomfromtext('POLYGON((10 10,12 10,12 12,10 12,10 10))')),
        (3, st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))
    )
),
b(id, geom) AS (
    SELECT * FROM (VALUES
        (10, st_geomfromtext('POLYGON((1 1,2 1,2 2,1 2,1 1))')),
        (20, st_geomfromtext('POLYGON((11 11,13 11,13 13,11 13,11 11))'))
    )
)
SELECT CASE WHEN count(*) = 3
            THEN 'PASS bbox join 3 matches' ELSE 'FAIL bbox join 3 matches' END
FROM a JOIN b
  ON st_xmin(a.geom) <= st_xmax(b.geom) AND st_xmax(a.geom) >= st_xmin(b.geom)
 AND st_ymin(a.geom) <= st_ymax(b.geom) AND st_ymax(a.geom) >= st_ymin(b.geom)
WHERE st_intersects(a.geom, b.geom);

-- ======================================================================
-- 4. Dissolve by category (ST_Union_Agg)
-- ======================================================================

WITH parcels(category, geom) AS (
    SELECT * FROM (VALUES
        ('residential', st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')),
        ('residential', st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))')),
        ('commercial', st_geomfromtext('POLYGON((0 1,1 1,1 2,0 2,0 1))'))
    )
)
SELECT CASE WHEN (SELECT count(*) FROM (
    SELECT category, st_area(st_union_agg(geom)) AS total_area
    FROM parcels GROUP BY category
)) = 2
            THEN 'PASS dissolve 2 categories' ELSE 'FAIL dissolve 2 categories' END;

SELECT CASE WHEN abs((SELECT st_area(st_union_agg(geom)) FROM (
    SELECT * FROM (VALUES
        ('a', st_geomfromtext('POLYGON((0 0,2 0,2 1,0 1,0 0))')),
        ('a', st_geomfromtext('POLYGON((1 0,3 0,3 1,1 1,1 0))'))
    ) AS t(cat, geom)
)) - 3.0) < 1e-9
            THEN 'PASS dissolve merge area' ELSE 'FAIL dissolve merge area' END;

-- ======================================================================
-- 5. Dump + DumpPoints + DumpRings
-- ======================================================================

-- Dump: explode MultiPolygon to individual polygons
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))'))) = 2
            THEN 'PASS dump multipolygon' ELSE 'FAIL dump multipolygon' END;

-- DumpPoints: all vertices of a polygon
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'))) >= 5
            THEN 'PASS dumppoints polygon' ELSE 'FAIL dumppoints polygon' END;

-- DumpRings: exterior + interior of a polygon with hole
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'))) = 2
            THEN 'PASS dumprings poly+hole' ELSE 'FAIL dumprings poly+hole' END;

-- ======================================================================
-- 6. Raster sampling + reclassification
-- ======================================================================

-- ST_Value: point sampling
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 0.5, 2.5) = 1.0
            THEN 'PASS raster value point' ELSE 'FAIL raster value point' END;

-- ST_PixelData: pixel streaming + SQL reclassification
SELECT CASE WHEN (SELECT count(*) FROM st_pixeldata('tests/data/test_raster.asc', 1)
    WHERE value > 5) = 7
            THEN 'PASS raster reclassify' ELSE 'FAIL raster reclassify' END;

-- ======================================================================
-- 7. Geodesic distance
-- ======================================================================

SELECT CASE WHEN abs(st_distancesphere(st_point(-0.1278, 51.5074),
                                        st_point(2.3522, 48.8566)) - 343520.0) < 1000.0
            THEN 'PASS geodesic sphere london_paris' ELSE 'FAIL geodesic sphere london_paris' END;

SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(1,0))
                  > st_distancesphere(st_point(0,0), st_point(1,0))
            THEN 'PASS geodesic spheroid >= sphere' ELSE 'FAIL geodesic spheroid >= sphere' END;

-- ======================================================================
-- 8. Overlay + GEOS fallback
-- ======================================================================

-- Valid polygon overlay
SELECT CASE WHEN abs(st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 0,6 0,6 4,2 4,2 0))'))) - 8.0) < 1e-9
            THEN 'PASS overlay valid intersection' ELSE 'FAIL overlay valid intersection' END;

-- GEOS topology: MakeValid + Node
SELECT CASE WHEN st_isvalid(st_makevalid(
    st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')))
            THEN 'PASS geos makevalid bowtie' ELSE 'FAIL geos makevalid bowtie' END;

-- ======================================================================
-- 9. PostGIS migration patterns
-- ======================================================================

-- PostGIS: ST_GeomFromText → works identically
SELECT CASE WHEN st_astext(st_geomfromtext('LINESTRING(0 0,1 1,2 2)'))
                 = 'LINESTRING(0 0,1 1,2 2)'
            THEN 'PASS migration geomfromtext' ELSE 'FAIL migration geomfromtext' END;

-- PostGIS: ST_DWithin → works identically
SELECT CASE WHEN st_dwithin(st_point(0,0), st_point(0,5), 10.0)
            THEN 'PASS migration dwithin' ELSE 'FAIL migration dwithin' END;

-- PostGIS: ST_Buffer + ST_Intersects chain
SELECT CASE WHEN st_intersects(
    st_buffer(st_point(0,0), 5.0),
    st_point(3,0))
            THEN 'PASS migration buffer_intersects' ELSE 'FAIL migration buffer_intersects' END;

-- PostGIS: ST_AsGeoJSON output
SELECT CASE WHEN st_asgeojson(st_point(1,2)) LIKE '%1%'
            THEN 'PASS migration asgeojson' ELSE 'FAIL migration asgeojson' END;

-- PostGIS: ST_Collect aggregate + ST_ConvexHull
SELECT CASE WHEN st_area(st_convexhull(st_collect(g))) > 0
            THEN 'PASS migration collect_hull' ELSE 'FAIL migration collect_hull' END
FROM (SELECT st_point(i, cast(i as double) * i % 5) AS g FROM range(0, 5) t(i));

-- PostGIS: ST_ForceCollection
SELECT CASE WHEN st_geometrytype(st_forcecollection(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'))) = 'ST_GeometryCollection'
            THEN 'PASS migration forcecollection' ELSE 'FAIL migration forcecollection' END;

-- ======================================================================
-- 10. Literal routing parity (st_* == sedona_st_*)
-- ======================================================================

SELECT CASE WHEN st_astext(st_point(1,2)) = sedona_st_astext(sedona_st_point(1,2))
            THEN 'PASS routing point parity' ELSE 'FAIL routing point parity' END;

SELECT CASE WHEN st_astext(st_translate(st_point(0,0), 3, 4)) =
                 sedona_st_astext(sedona_st_translate(st_point(0,0), 3, 4))
            THEN 'PASS routing translate parity' ELSE 'FAIL routing translate parity' END;
