-- postgis_accessors.sql — curated from PostGIS regress: iscollection,
-- hausdorff, minimumclearance.
-- Source: regress/core/{iscollection,hausdorff,minimum_clearance}.sql
.mode list

-- ======================================================================
-- ST_IsCollection — true for MULTI* and GEOMETRYCOLLECTION
-- ======================================================================

-- Singletons are NOT collections
SELECT CASE WHEN st_iscollection(st_geomfromtext('POINT(0 0)')) = false
            THEN 'PASS iscoll point' ELSE 'FAIL iscoll point' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext('LINESTRING(0 0,1 1)')) = false
            THEN 'PASS iscoll line' ELSE 'FAIL iscoll line' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))')) = false
            THEN 'PASS iscoll polygon' ELSE 'FAIL iscoll polygon' END;

-- Multi types ARE collections
SELECT CASE WHEN st_iscollection(st_geomfromtext('MULTIPOINT((0 0),(1 1))')) = true
            THEN 'PASS iscoll multipoint' ELSE 'FAIL iscoll multipoint' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext('MULTILINESTRING((0 0,1 1))')) = true
            THEN 'PASS iscoll multiline' ELSE 'FAIL iscoll multiline' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext(
    'MULTIPOLYGON(((0 0,1 0,1 1,0 0)))'
)) = true
            THEN 'PASS iscoll multipolygon' ELSE 'FAIL iscoll multipolygon' END;

-- Empty multi types are still collections
SELECT CASE WHEN st_iscollection(st_geomfromtext('MULTIPOINT EMPTY')) = true
            THEN 'PASS iscoll empty_multipoint' ELSE 'FAIL iscoll empty_multipoint' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext('MULTILINESTRING EMPTY')) = true
            THEN 'PASS iscoll empty_multiline' ELSE 'FAIL iscoll empty_multiline' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext('MULTIPOLYGON EMPTY')) = true
            THEN 'PASS iscoll empty_multipoly' ELSE 'FAIL iscoll empty_multipoly' END;

-- GeometryCollection IS a collection
SELECT CASE WHEN st_iscollection(st_geomfromtext(
    'GEOMETRYCOLLECTION(POINT(0 0))'
)) = true
            THEN 'PASS iscoll collection' ELSE 'FAIL iscoll collection' END;
SELECT CASE WHEN st_iscollection(st_geomfromtext(
    'GEOMETRYCOLLECTION EMPTY'
)) = true
            THEN 'PASS iscoll empty_collection' ELSE 'FAIL iscoll empty_collection' END;

-- Nested collection
SELECT CASE WHEN st_iscollection(st_geomfromtext(
    'GEOMETRYCOLLECTION(POINT(0 0),LINESTRING(0 0,1 1))'
)) = true
            THEN 'PASS iscoll nested_collection' ELSE 'FAIL iscoll nested_collection' END;

-- ======================================================================
-- ST_HausdorffDistance — discrete Hausdorff distance
-- ======================================================================

-- Identical geometries → 0
SELECT CASE WHEN st_hausdorffdistance(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('LINESTRING(0 0,1 1)')
) = 0.0
THEN 'PASS hausdorff identical' ELSE 'FAIL hausdorff identical' END;

-- Different lines: distance is the max deviation
SELECT CASE WHEN st_hausdorffdistance(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('LINESTRING(0 0,2 2)')
) > 0.0
THEN 'PASS hausdorff different' ELSE 'FAIL hausdorff different' END;

-- Polygon to polygon
SELECT CASE WHEN st_hausdorffdistance(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')
) > 0.0
THEN 'PASS hausdorff polygons' ELSE 'FAIL hausdorff polygons' END;

-- Line to multipoint
SELECT CASE WHEN st_hausdorffdistance(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('MULTIPOINT((0 0),(2 2))')
) >= 0.0
THEN 'PASS hausdorff line_multipoint' ELSE 'FAIL hausdorff line_multipoint' END;

-- ======================================================================
-- ST_MinimumClearance — the minimum clearance of a geometry
-- ======================================================================

-- Square has clearance equal to the shortest side
SELECT CASE WHEN st_minimumclearance(st_geomfromtext(
    'POLYGON((0 0,1 0,1 1,0 1,0 0))'
)) IS NOT NULL
THEN 'PASS minclear polygon' ELSE 'FAIL minclear polygon' END;

-- Point clearance
SELECT CASE WHEN st_minimumclearance(st_geomfromtext(
    'POINT(0 0)'
)) IS NOT NULL
THEN 'PASS minclear point' ELSE 'FAIL minclear point' END;

-- ======================================================================
-- ST_IsValid — boolean validity check (bonus from regress_ogc)
-- ======================================================================

-- Valid square
SELECT CASE WHEN st_isvalid(st_geomfromtext(
    'POLYGON((0 0,10 0,10 10,0 10,0 0))'
)) = true
THEN 'PASS isvalid square' ELSE 'FAIL isvalid square' END;

-- Invalid bowtie
SELECT CASE WHEN st_isvalid(st_geomfromtext(
    'POLYGON((0 0,1 1,1 0,0 1,0 0))'
)) = false
THEN 'PASS isvalid bowtie' ELSE 'FAIL isvalid bowtie' END;

-- Valid triangle
SELECT CASE WHEN st_isvalid(st_geomfromtext(
    'POLYGON((0 0,1 0,0.5 1,0 0))'
)) = true
THEN 'PASS isvalid triangle' ELSE 'FAIL isvalid triangle' END;
