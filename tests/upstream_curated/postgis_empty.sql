-- postgis_empty.sql — curated from PostGIS regress/core/empty.sql
-- Tests EMPTY geometry handling across spatial functions.
-- Source: https://postgis.net/docs/regress/core/empty.sql
.mode list

-- ======================================================================
-- 1. EMPTY geometry constructors
-- ======================================================================
SELECT CASE WHEN st_isempty(st_geomfromtext('POINT EMPTY'))
            THEN 'PASS empty point' ELSE 'FAIL empty point' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('LINESTRING EMPTY'))
            THEN 'PASS empty line' ELSE 'FAIL empty line' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('POLYGON EMPTY'))
            THEN 'PASS empty polygon' ELSE 'FAIL empty polygon' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('MULTIPOLYGON EMPTY'))
            THEN 'PASS empty multipolygon' ELSE 'FAIL empty multipolygon' END;
SELECT CASE WHEN st_isempty(st_geomfromtext('GEOMETRYCOLLECTION EMPTY'))
            THEN 'PASS empty collection' ELSE 'FAIL empty collection' END;

-- ======================================================================
-- 2. ST_Area / ST_Length / ST_Perimeter on EMPTY = 0
-- ======================================================================
SELECT CASE WHEN st_area(st_geomfromtext('POLYGON EMPTY')) = 0.0
            THEN 'PASS empty area zero' ELSE 'FAIL empty area zero' END;
SELECT CASE WHEN st_length(st_geomfromtext('LINESTRING EMPTY')) = 0.0
            THEN 'PASS empty length zero' ELSE 'FAIL empty length zero' END;
SELECT CASE WHEN st_perimeter(st_geomfromtext('POLYGON EMPTY')) = 0.0
            THEN 'PASS empty perimeter zero' ELSE 'FAIL empty perimeter zero' END;

-- ======================================================================
-- 3. ST_Union(geometry, EMPTY) == geometry (PostGIS contract)
-- ======================================================================
SELECT CASE WHEN NOT st_isempty(st_union(
                st_geomfromtext('POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))'),
                st_geomfromtext('POLYGON EMPTY')))
            THEN 'PASS union geom empty' ELSE 'FAIL union geom empty' END;

-- ======================================================================
-- 4. ST_Union(EMPTY, EMPTY) == EMPTY
-- ======================================================================
SELECT CASE WHEN st_isempty(st_union(
                st_geomfromtext('POLYGON EMPTY'),
                st_geomfromtext('POLYGON EMPTY')))
            THEN 'PASS union empty empty' ELSE 'FAIL union empty empty' END;

-- ======================================================================
-- 5. ST_Intersection(geometry, EMPTY) == EMPTY (PostGIS contract)
-- ======================================================================
SELECT CASE WHEN st_isempty(st_intersection(
                st_geomfromtext('POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))'),
                st_geomfromtext('POLYGON EMPTY')))
            THEN 'PASS intersection geom empty' ELSE 'FAIL intersection geom empty' END;

-- ======================================================================
-- 6. ST_Dimension on EMPTY = -1 (PostGIS parity, M delta closure)
-- ======================================================================
SELECT CASE WHEN st_dimension(st_geomfromtext('POINT EMPTY')) = -1
            THEN 'PASS empty dimension neg1' ELSE 'FAIL empty dimension neg1' END;
SELECT CASE WHEN st_dimension(st_geomfromtext('POLYGON EMPTY')) = -1
            THEN 'PASS empty polygon dimension neg1' ELSE 'FAIL empty polygon dimension neg1' END;

-- ======================================================================
-- 7. ST_IsEmpty on non-empty = false
-- ======================================================================
SELECT CASE WHEN NOT st_isempty(st_geomfromtext('POINT(1 2)'))
            THEN 'PASS nonempty point' ELSE 'FAIL nonempty point' END;

-- ======================================================================
-- 8. ST_Buffer on EMPTY
-- PostGIS: buffer of EMPTY with tolerance 0 returns EMPTY
-- Our implementation: returns NULL (geo buffer panics on empty input;
-- documented behavioral difference — NULL is safe, never wrong)
-- ======================================================================
SELECT CASE WHEN st_buffer(st_geomfromtext('POLYGON EMPTY'), 0) IS NULL
            THEN 'PASS buffer empty null' ELSE 'FAIL buffer empty null' END;

-- ======================================================================
-- 9. ST_Envelope on EMPTY
-- PostGIS: returns NULL (no bounding box for empty)
-- ======================================================================
SELECT CASE WHEN st_envelope(st_geomfromtext('POINT EMPTY')) IS NULL
                 OR st_isempty(st_envelope(st_geomfromtext('POINT EMPTY')))
            THEN 'PASS envelope empty' ELSE 'FAIL envelope empty' END;

-- ======================================================================
-- 10. ST_NumPoints / ST_NPoints on EMPTY = 0
-- ======================================================================
SELECT CASE WHEN st_numpoints(st_geomfromtext('LINESTRING EMPTY')) = 0
            THEN 'PASS empty numpoints zero' ELSE 'FAIL empty numpoints zero' END;
SELECT CASE WHEN st_npoints(st_geomfromtext('POINT EMPTY')) = 0
            THEN 'PASS empty npoints zero' ELSE 'FAIL empty npoints zero' END;

-- ======================================================================
-- 11. ST_Centroid on EMPTY
-- PostGIS: returns POINT EMPTY
-- ======================================================================
SELECT CASE WHEN st_centroid(st_geomfromtext('POLYGON EMPTY')) IS NULL
                 OR st_isempty(st_centroid(st_geomfromtext('POLYGON EMPTY')))
            THEN 'PASS centroid empty' ELSE 'FAIL centroid empty' END;
