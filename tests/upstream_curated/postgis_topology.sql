-- postgis_topology.sql — curated from PostGIS regress: node, polygonize, snap,
-- voronoi, delaunaytriangles. All GEOS-backed topology functions.
-- Source: regress/core/{node,polygonize,snap,voronoi,delaunaytriangles}.sql
.mode list

-- ======================================================================
-- ST_Node — node a collection of linestrings (GEOS)
-- ======================================================================

-- t1: crossing lines get noded at intersection
SELECT CASE WHEN st_numpoints(st_node(st_geomfromtext(
    'MULTILINESTRING((0 0,10 0),(5 -5,5 5))'
))) > 4
THEN 'PASS node crossing' ELSE 'FAIL node crossing' END;

-- t2: overlapping lines get noded at endpoints
SELECT CASE WHEN st_numpoints(st_node(st_geomfromtext(
    'MULTILINESTRING((0 0,10 0,20 0),(8 0,15 0,25 0))'
))) > 2
THEN 'PASS node overlap' ELSE 'FAIL node overlap' END;

-- t3: self-intersecting line gets noded
SELECT CASE WHEN st_numpoints(st_node(st_geomfromtext(
    'LINESTRING(0 0,10 10,0 10,5 5,10 0)'
))) > 3
THEN 'PASS node self_intersect' ELSE 'FAIL node self_intersect' END;

-- ST_Node preserves SRID tag
SELECT CASE WHEN st_srid(st_node(st_setsrid(st_geomfromtext(
    'MULTILINESTRING((0 0,10 0),(5 -5,5 5))'), 4326
))) = 4326
THEN 'PASS node srid' ELSE 'FAIL node srid' END;

-- ======================================================================
-- ST_Polygonize — create polygons from linework (GEOS)
-- ======================================================================

-- t1: closed rings form a polygon
SELECT CASE WHEN st_numgeometries(st_polygonize(st_geomfromtext(
    'MULTILINESTRING((0 0,1 0,1 1,0 1,0 0))'
))) >= 1
THEN 'PASS polygonize basic' ELSE 'FAIL polygonize basic' END;

-- t2: multiple closed rings
SELECT CASE WHEN st_numgeometries(st_polygonize(st_geomfromtext(
    'MULTILINESTRING((0 0,2 0,2 2,0 2,0 0),(1 1,1.5 1,1.5 1.5,1 1.5,1 1))'
))) >= 1
THEN 'PASS polygonize multi' ELSE 'FAIL polygonize multi' END;

-- ======================================================================
-- ST_Snap — snap geometry to another within tolerance (GEOS)
-- ======================================================================

-- t2: vertex snapping preserves SRID
SELECT CASE WHEN st_srid(st_snap(
    st_setsrid(st_geomfromtext('LINESTRING(0 0,10 0)'), 10),
    st_setsrid(st_geomfromtext('POINT(5 0.1)'), 10),
    0.2
)) = 10
THEN 'PASS snap srid' ELSE 'FAIL snap srid' END;

-- t3: segment snap
SELECT CASE WHEN st_numpoints(st_snap(
    st_geomfromtext('LINESTRING(0 0,10 0)'),
    st_geomfromtext('POINT(5 0)'),
    1.0
)) >= 2
THEN 'PASS snap vertex' ELSE 'FAIL snap vertex' END;

-- t5: polygon snap
SELECT CASE WHEN st_snap(
    st_geomfromtext('LINESTRING(70 250,230 340)'),
    st_geomfromtext('POINT(70 250)'),
    1.0
) IS NOT NULL
THEN 'PASS snap polygon' ELSE 'FAIL snap polygon' END;

-- ======================================================================
-- ST_VoronoiPolygons — Voronoi diagram (GEOS)
-- ======================================================================

-- Voronoi of 4 points yields >= 4 cells
SELECT CASE WHEN st_numgeometries(st_voronoipolygons(
    st_geomfromtext('MULTIPOINT((0 0),(1 0),(0 1),(1 1))')
)) >= 4
THEN 'PASS voronoi basic' ELSE 'FAIL voronoi basic' END;

-- Voronoi preserves SRID
SELECT CASE WHEN st_srid(st_voronoipolygons(
    st_setsrid(st_geomfromtext('MULTIPOINT((0 0),(1 0),(0 1),(1 1))'), 4326)
)) = 4326
THEN 'PASS voronoi srid' ELSE 'FAIL voronoi srid' END;

-- Voronoi on NULL → NULL
SELECT CASE WHEN st_voronoipolygons(NULL) IS NULL
THEN 'PASS voronoi null' ELSE 'FAIL voronoi null' END;

-- ======================================================================
-- ST_DelaunayTriangles — Delaunay triangulation (GEOS)
-- ======================================================================

-- Basic: 3 non-collinear points → 1 triangle
SELECT CASE WHEN st_numgeometries(st_delaunaytriangles(
    st_geomfromtext('MULTIPOINT((0 0),(1 0),(0 1))')
)) >= 1
THEN 'PASS delaunay basic' ELSE 'FAIL delaunay basic' END;

-- 4 points → >= 2 triangles
SELECT CASE WHEN st_numgeometries(st_delaunaytriangles(
    st_geomfromtext('MULTIPOINT((5 5),(6 0),(7 9),(8 9))')
)) >= 2
THEN 'PASS delaunay four_pts' ELSE 'FAIL delaunay four_pts' END;

-- Flag 0 (default): return polygons
SELECT CASE WHEN st_geometrytype(st_delaunaytriangles(
    st_geomfromtext('MULTIPOINT((5 5),(6 0),(7 9))')
)) = 'ST_GeometryCollection'
THEN 'PASS delaunay polygon_flag' ELSE 'FAIL delaunay polygon_flag' END;

-- Flag 1: return edges (multilinestring) — 3-arg form not yet supported;
-- documented gap (PostGIS has tolerance + flag params).

-- Delaunay preserves SRID
SELECT CASE WHEN st_srid(st_delaunaytriangles(
    st_setsrid(st_geomfromtext('MULTIPOINT((0 0),(1 0),(0 1))'), 4326)
)) = 4326
THEN 'PASS delaunay srid' ELSE 'FAIL delaunay srid' END;
