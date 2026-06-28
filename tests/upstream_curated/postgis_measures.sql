-- postgis_measures.sql — curated from PostGIS regress/core/measures.sql
-- Tests ST_Area, ST_Length, ST_Perimeter, ST_Distance, ST_MaxDistance
-- against PostGIS expected output.
-- Source: https://postgis.net/docs/regress/core/measures.sql
.mode list

-- ======================================================================
-- 1. ST_Area of a simple square = 100
-- ======================================================================
SELECT CASE WHEN st_area(st_geomfromtext('POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))')) = 100.0
            THEN 'PASS area square' ELSE 'FAIL area square' END;

-- ======================================================================
-- 2. ST_Area of a polygon with a hole (outer 10x10 minus inner 2x2 = 96)
-- ======================================================================
SELECT CASE WHEN st_area(st_geomfromtext(
    'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5))'
)) = 96.0
THEN 'PASS area with hole' ELSE 'FAIL area with hole' END;

-- ======================================================================
-- 3. ST_Area of MULTIPOLYGON (two 10x10 squares = 200)
-- ======================================================================
SELECT CASE WHEN st_area(st_geomfromtext(
    'MULTIPOLYGON(((0 0, 10 0, 10 10, 0 10, 0 0)),((20 0, 30 0, 30 10, 20 10, 20 0)))'
)) = 200.0
THEN 'PASS area multipolygon' ELSE 'FAIL area multipolygon' END;

-- ======================================================================
-- 4. ST_Perimeter of a square = 40
-- ======================================================================
SELECT CASE WHEN st_perimeter(st_geomfromtext(
    'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))'
)) = 40.0
THEN 'PASS perimeter square' ELSE 'FAIL perimeter square' END;

-- ======================================================================
-- 5. ST_Perimeter including inner ring (outer 40 + inner 8 = 48)
-- ======================================================================
SELECT CASE WHEN st_perimeter(st_geomfromtext(
    'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5))'
)) = 48.0
THEN 'PASS perimeter with hole' ELSE 'FAIL perimeter with hole' END;

-- ======================================================================
-- 6. ST_Length of a diagonal line = sqrt(2) ≈ 1.414
-- ======================================================================
SELECT CASE WHEN abs(st_length(st_geomfromtext('LINESTRING(0 0, 1 1)')) - 1.4142135623730951) < 1e-12
            THEN 'PASS length diagonal' ELSE 'FAIL length diagonal' END;

-- ======================================================================
-- 7. ST_Length of MULTILINESTRING
-- PostGIS: 4.242641 (two diagonals + one unit)
-- ======================================================================
SELECT CASE WHEN abs(st_length(st_geomfromtext(
    'MULTILINESTRING((0 0, 1 1),(0 0, 1 1, 2 2))'
)) - 4.242640687119286) < 1e-6
THEN 'PASS length multiline' ELSE 'FAIL length multiline' END;

-- ======================================================================
-- 8. ST_Distance: identical points = 0
-- ======================================================================
SELECT CASE WHEN st_distance(st_point(1, 2), st_point(1, 2)) = 0.0
            THEN 'PASS distance identical' ELSE 'FAIL distance identical' END;

-- ======================================================================
-- 9. ST_Distance: 5-12-13 triangle = 13
-- ======================================================================
SELECT CASE WHEN st_distance(st_point(5, 0), st_point(10, 12)) = 13.0
            THEN 'PASS distance 5_12_13' ELSE 'FAIL distance 5_12_13' END;

-- ======================================================================
-- 10. ST_Distance between disjoint polygons
-- PostGIS 'dist': distance between adjacent polygons = 1 (gap from x=10 to x=11)
-- ======================================================================
SELECT CASE WHEN st_distance(
    st_geomfromtext('POLYGON((0 0, 0 10, 10 10, 10 0, 0 0))'),
    st_geomfromtext('POLYGON((11 0, 11 10, 20 10, 20 0, 11 0),(15 5, 15 8, 17 8, 17 5, 15 5))')
) = 1.0
THEN 'PASS distance polygons' ELSE 'FAIL distance polygons' END;

-- ======================================================================
-- 11. ST_Distance is symmetric (dist(a,b) == dist(b,a))
-- ======================================================================
SELECT CASE WHEN st_distance(
    st_geomfromtext('POLYGON((0 0, 0 10, 10 10, 10 0, 0 0))'),
    st_geomfromtext('POLYGON((11 0, 11 10, 20 10, 20 0, 11 0))')
) = st_distance(
    st_geomfromtext('POLYGON((11 0, 11 10, 20 10, 20 0, 11 0))'),
    st_geomfromtext('POLYGON((0 0, 0 10, 10 10, 10 0, 0 0))')
)
THEN 'PASS distance symmetric' ELSE 'FAIL distance symmetric' END;

-- ======================================================================
-- 12. ST_DWithin: adjacent polygons within distance 1.5
-- ======================================================================
SELECT CASE WHEN st_dwithin(
    st_geomfromtext('POLYGON((0 0, 0 10, 10 10, 10 0, 0 0))'),
    st_geomfromtext('POLYGON((11 0, 11 10, 20 10, 20 0, 11 0))'),
    1.5
) = true
THEN 'PASS dwithin true' ELSE 'FAIL dwithin true' END;

-- ======================================================================
-- 13. ST_DWithin: not within distance 0.5
-- ======================================================================
SELECT CASE WHEN st_dwithin(
    st_geomfromtext('POLYGON((0 0, 0 10, 10 10, 10 0, 0 0))'),
    st_geomfromtext('POLYGON((11 0, 11 10, 20 10, 20 0, 11 0))'),
    0.5
) = false
THEN 'PASS dwithin false' ELSE 'FAIL dwithin false' END;

-- ======================================================================
-- 14. ST_Area of large multipolygon with overlapping parts
-- PostGIS test 113: three overlapping squares, each 10x10 = 300
-- (union area would be 100, but ST_Area counts overlapping areas)
-- ======================================================================
SELECT CASE WHEN st_area(st_geomfromtext(
    'MULTIPOLYGON(((0 0, 10 0, 10 10, 0 10, 0 0)),((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5)),((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5),(1 1, 2 1, 2 2, 1 2, 1 1)))'
)) = 291.0
THEN 'PASS area overlapping multipolygon' ELSE 'FAIL area overlapping multipolygon' END;

-- ======================================================================
-- 15. ST_Perimeter of the same overlapping multipolygon
-- PostGIS test 114: 40 + (40+8) + (40+8+4) = 140
-- ======================================================================
SELECT CASE WHEN st_perimeter(st_geomfromtext(
    'MULTIPOLYGON(((0 0, 10 0, 10 10, 0 10, 0 0)),((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5)),((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 7 5, 7 7, 5 7, 5 5),(1 1, 2 1, 2 2, 1 2, 1 1)))'
)) = 140.0
THEN 'PASS perimeter overlapping multipolygon' ELSE 'FAIL perimeter overlapping multipolygon' END;
