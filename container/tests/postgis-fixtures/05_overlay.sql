-- PostGIS fixture: set operations and overlay
-- Tests intersection, union, difference through the facade.

SELECT CASE WHEN abs(st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))')) - 1.0) < 0.001
            THEN 'PASS overlay intersection' ELSE 'FAIL overlay intersection' END;

SELECT CASE WHEN abs(st_area(st_union(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 0,3 0,3 2,1 2,1 0))'))) - 6.0) < 0.001
            THEN 'PASS overlay union' ELSE 'FAIL overlay union' END;

SELECT CASE WHEN abs(st_area(st_difference(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 12.0) < 0.001
            THEN 'PASS overlay difference' ELSE 'FAIL overlay difference' END;

SELECT CASE WHEN abs(st_area(st_symdifference(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 0,3 0,3 2,1 2,1 0))'))) - 4.0) < 0.001
            THEN 'PASS overlay symdifference' ELSE 'FAIL overlay symdifference' END;
