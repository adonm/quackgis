-- PostGIS fixture: predicates and DE-9IM
-- Tests spatial predicates through the QuackGIS facade.

SELECT CASE WHEN st_intersects(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))
            THEN 'PASS predicate intersects' ELSE 'FAIL predicate intersects' END;

SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(1 1)'))
            THEN 'PASS predicate contains' ELSE 'FAIL predicate contains' END;

SELECT CASE WHEN st_within(
    st_geomfromtext('POINT(1 1)'),
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'))
            THEN 'PASS predicate within' ELSE 'FAIL predicate within' END;

SELECT CASE WHEN st_disjoint(
    st_geomfromtext('POINT(0 0)'),
    st_geomfromtext('POINT(10 10)'))
            THEN 'PASS predicate disjoint' ELSE 'FAIL predicate disjoint' END;

SELECT CASE WHEN st_dwithin(
    st_geomfromtext('POINT(0 0)'),
    st_geomfromtext('POINT(3 4)'), 6.0)
            THEN 'PASS predicate dwithin' ELSE 'FAIL predicate dwithin' END;

SELECT CASE WHEN NOT st_disjoint(
    st_geomfromtext('LINESTRING(0 0,4 4)'),
    st_geomfromtext('LINESTRING(0 4,4 0)'))
            THEN 'PASS predicate crosses' ELSE 'FAIL predicate crosses' END;

SELECT CASE WHEN st_covers(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(0 0)'))
            THEN 'PASS predicate covers' ELSE 'FAIL predicate covers' END;
