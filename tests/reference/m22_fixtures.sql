.mode list
-- Month 22: ST_Contains / ST_Within boundary delta retirement.
--
-- M22 fix: both predicates route through geo::Relate with PostGIS DE-9IM
-- pattern T*****FF* instead of the old PNPOLY ray-cast. Boundary points
-- now return FALSE (matching PostGIS), interior points still return TRUE.

-- Interior point: contains → TRUE (unchanged)
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(1 1)'))
THEN 'PASS m22_contains_interior' ELSE 'FAIL m22_contains_interior' END;

-- Interior point: within → TRUE (unchanged)
SELECT CASE WHEN st_within(
    st_geomfromtext('POINT(1 1)'),
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS m22_within_interior' ELSE 'FAIL m22_within_interior' END;

-- Boundary vertex: contains → FALSE (was TRUE before M22)
SELECT CASE WHEN NOT st_contains(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(0 0)'))
THEN 'PASS m22_contains_vertex_false' ELSE 'FAIL m22_contains_vertex_false' END;

-- Boundary on edge midpoint: contains → FALSE (was TRUE before M22)
SELECT CASE WHEN NOT st_contains(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(2 0)'))
THEN 'PASS m22_contains_edge_false' ELSE 'FAIL m22_contains_edge_false' END;

-- Boundary vertex: within → FALSE (was TRUE before M22)
SELECT CASE WHEN NOT st_within(
    st_geomfromtext('POINT(0 0)'),
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS m22_within_vertex_false' ELSE 'FAIL m22_within_vertex_false' END;

-- Exterior point: contains → FALSE (unchanged)
SELECT CASE WHEN NOT st_contains(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(10 10)'))
THEN 'PASS m22_contains_exterior' ELSE 'FAIL m22_contains_exterior' END;

-- Polygon contains smaller polygon → TRUE (interior intersection)
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,0 10,10 10,10 0,0 0))'),
    st_geomfromtext('POLYGON((2 2,2 8,8 8,8 2,2 2))'))
THEN 'PASS m22_contains_polygon' ELSE 'FAIL m22_contains_polygon' END;

-- Polygon contains polygon sharing an edge → TRUE (interior still intersects,
-- boundary contact is allowed — boundary delta only applies when the ENTIRE
-- second geometry lies only on the boundary, e.g. a boundary point)
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,0 10,10 10,10 0,0 0))'),
    st_geomfromtext('POLYGON((0 0,0 5,5 5,5 0,0 0))'))
THEN 'PASS m22_contains_shared_edge_true' ELSE 'FAIL m22_contains_shared_edge_true' END;

-- ST_Covers boundary vertex → TRUE (unchanged, correct boundary-inclusive)
SELECT CASE WHEN st_covers(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(0 0)'))
THEN 'PASS m22_covers_boundary_true' ELSE 'FAIL m22_covers_boundary_true' END;

-- ST_ContainsProperly boundary vertex → FALSE (unchanged)
SELECT CASE WHEN NOT st_containsproperly(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT(0 0)'))
THEN 'PASS m22_containsproperly_boundary' ELSE 'FAIL m22_containsproperly_boundary' END;

-- NULL propagation
SELECT CASE WHEN st_contains(NULL, st_geomfromtext('POINT(0 0)')) IS NULL
THEN 'PASS m22_contains_null' ELSE 'FAIL m22_contains_null' END;

SELECT CASE WHEN st_within(st_geomfromtext('POINT(0 0)'), NULL) IS NULL
THEN 'PASS m22_within_null' ELSE 'FAIL m22_within_null' END;

-- Empty operand
SELECT CASE WHEN NOT st_contains(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POINT EMPTY'))
THEN 'PASS m22_contains_empty' ELSE 'FAIL m22_contains_empty' END;
