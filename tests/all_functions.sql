-- SPDX-License-Identifier: Apache-2.0
-- Comprehensive SQL test: exercises every registered ST_* function with known
-- inputs and expected outputs. Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/all_functions.sql
--
-- Each test prints 'PASS' or 'FAIL <details>'. If any FAIL appears, investigate.
.bail off
.mode list

-- === Constructors ===
SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)' THEN 'PASS' ELSE 'FAIL geomfromtext' END;
SELECT CASE WHEN st_astext(st_point(3, 4)) = 'POINT(3 4)' THEN 'PASS' ELSE 'FAIL point' END;
SELECT CASE WHEN st_geometrytype(st_geomfromtext('LINESTRING(0 0,1 1)')) = 'ST_LineString' THEN 'PASS' ELSE 'FAIL geometrytype' END;

-- === Measurements ===
SELECT CASE WHEN st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = 16.0 THEN 'PASS' ELSE 'FAIL area' END;
SELECT CASE WHEN st_length(st_geomfromtext('LINESTRING(0 0,3 0,3 4)')) = 7.0 THEN 'PASS' ELSE 'FAIL length' END;
SELECT CASE WHEN st_distance(st_geomfromtext('POINT(0 0)'), st_geomfromtext('POINT(3 4)')) = 5.0 THEN 'PASS' ELSE 'FAIL distance' END;
SELECT CASE WHEN round(st_perimeter(st_geomfromtext('POLYGON((0 0,3 0,3 3,0 3,0 0))')),0) = 12.0 THEN 'PASS' ELSE 'FAIL perimeter' END;

-- === Predicates ===
SELECT CASE WHEN st_intersects(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), st_point(1,1)) = true THEN 'PASS' ELSE 'FAIL intersects' END;
SELECT CASE WHEN st_contains(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), st_point(3,3)) = false THEN 'PASS' ELSE 'FAIL contains' END;
SELECT CASE WHEN st_within(st_point(1,1), st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')) = true THEN 'PASS' ELSE 'FAIL within' END;
SELECT CASE WHEN st_disjoint(st_point(9,9), st_point(0,0)) = true THEN 'PASS' ELSE 'FAIL disjoint' END;
SELECT CASE WHEN st_dwithin(st_point(0,0), st_point(1,0), 2.0) = true THEN 'PASS' ELSE 'FAIL dwithin' END;
SELECT CASE WHEN st_equals(st_point(1,1), st_point(1,1)) = true THEN 'PASS' ELSE 'FAIL equals' END;
SELECT CASE WHEN st_touches(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'), st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))')) = true THEN 'PASS' ELSE 'FAIL touches' END;
SELECT CASE WHEN st_covers(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), st_point(1,1)) = true THEN 'PASS' ELSE 'FAIL covers' END;

-- === Set ops ===
SELECT CASE WHEN st_area(st_intersection(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) = 1.0 THEN 'PASS' ELSE 'FAIL intersection' END;
SELECT CASE WHEN st_area(st_union(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'), st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))'))) = 2.0 THEN 'PASS' ELSE 'FAIL union' END;

-- === Transforms ===
SELECT CASE WHEN st_astext(st_buffer(st_point(0,0), 1.0)) IS NOT NULL THEN 'PASS' ELSE 'FAIL buffer' END;
SELECT CASE WHEN st_length(st_simplify(st_geomfromtext('LINESTRING(0 0,0.01 0,1 0)'), 0.1)) <= 1.0 THEN 'PASS' ELSE 'FAIL simplify' END;
SELECT CASE WHEN st_astext(st_translate(st_point(1,2),5,5)) = 'POINT(6 7)' THEN 'PASS' ELSE 'FAIL translate' END;
SELECT CASE WHEN st_astext(st_scale(st_geomfromtext('POINT(1 1)'),2,3)) IS NOT NULL THEN 'PASS' ELSE 'FAIL scale' END;
SELECT CASE WHEN round(st_x(st_centroid(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'))),1) = 2.0 THEN 'PASS' ELSE 'FAIL centroid' END;

-- === Accessors ===
SELECT CASE WHEN st_x(st_point(3,4)) = 3.0 THEN 'PASS' ELSE 'FAIL x' END;
SELECT CASE WHEN st_y(st_point(3,4)) = 4.0 THEN 'PASS' ELSE 'FAIL y' END;
SELECT CASE WHEN st_dimension(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))')) = 2 THEN 'PASS' ELSE 'FAIL dimension' END;
SELECT CASE WHEN st_numpoints(st_geomfromtext('LINESTRING(0 0,1 1,2 2)')) = 3 THEN 'PASS' ELSE 'FAIL numpoints' END;
SELECT CASE WHEN st_isvalid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) = true THEN 'PASS' ELSE 'FAIL isvalid' END;
SELECT CASE WHEN st_isempty(st_point(1,1)) = false THEN 'PASS' ELSE 'FAIL isempty' END;
SELECT CASE WHEN st_isclosed(st_geomfromtext('LINESTRING(0 0,1 1,0 0)')) = true THEN 'PASS' ELSE 'FAIL isclosed' END;

-- === CRS / Geography ===
SELECT CASE WHEN round(st_x(st_transform(st_point(0.1278,51.5074),4326,3857)),0) = 14227 THEN 'PASS' ELSE 'FAIL transform' END;
SELECT CASE WHEN round(st_distancesphere(st_geomfromtext('POINT(0 0)'),st_geomfromtext('POINT(0 1)')),-3) = 111000 THEN 'PASS' ELSE 'FAIL distancesphere' END;

-- === Delaunay / Voronoi ===
SELECT CASE WHEN st_numgeometries(st_delaunaytriangles(st_geomfromtext('MULTIPOINT(0 0,1 0,0 1,1 1,0.5 0.5)'))) >= 3 THEN 'PASS' ELSE 'FAIL delaunay' END;
SELECT CASE WHEN st_numgeometries(st_voronoilines(st_geomfromtext('MULTIPOINT(0 0,4 0,2 4,1 1)'))) >= 1 THEN 'PASS' ELSE 'FAIL voronoi' END;

-- === I/O ===
SELECT CASE WHEN st_asgeojson(st_point(1,2)) = '{"type":"Point","coordinates":[1,2]}' THEN 'PASS' ELSE 'FAIL asgeojson' END;
SELECT CASE WHEN st_asewkt(st_point(1,2),4326) = 'SRID=4326;POINT(1 2)' THEN 'PASS' ELSE 'FAIL asewkt' END;
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('LINESTRING(0 0,1 1,2 2)'))) = 'MULTIPOINT((0 0),(2 2))' THEN 'PASS' ELSE 'FAIL boundary' END;

-- === Aggregates ===
SELECT CASE WHEN st_numgeometries(st_collect(g)) = 3 THEN 'PASS' ELSE 'FAIL collect' END FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(1,1) UNION ALL SELECT st_point(2,2));
SELECT CASE WHEN st_astext(st_envelope_agg(g)) = 'POLYGON((0 0,5 0,5 8,0 8,0 0))' THEN 'PASS' ELSE 'FAIL envelope_agg' END FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(5,2) UNION ALL SELECT st_point(2,8));
SELECT CASE WHEN st_area(st_union_agg(g)) = 2.0 THEN 'PASS' ELSE 'FAIL union_agg' END FROM (SELECT st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))') g UNION ALL SELECT st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))'));

-- === Raster ===
SELECT CASE WHEN count = 16 THEN 'PASS' ELSE 'FAIL raster_stats' END FROM st_raster_stats('/var/home/adonm/dev/duckdb_sedona/build/raster/test.tif', 1);

-- === Tier 1/1b parity batch (editing, transforms, measurements, I/O) ===
SELECT CASE WHEN st_astext(st_affine(st_point(1,2),1,0,0,1,5,5)) = 'POINT(6 7)' THEN 'PASS' ELSE 'FAIL affine' END;
SELECT CASE WHEN st_numpoints(st_segmentize(st_geomfromtext('LINESTRING(0 0,10 0)'),4.0)) = 4 THEN 'PASS' ELSE 'FAIL segmentize' END;
SELECT CASE WHEN round(st_length(st_linesubstring(st_geomfromtext('LINESTRING(0 0,10 0)'),0.25,0.75)),4) = 5.0 THEN 'PASS' ELSE 'FAIL linesubstring' END;
SELECT CASE WHEN st_numgeometries(st_linemerge(st_geomfromtext('MULTILINESTRING((0 0,1 0),(1 0,2 0))'))) = 1 THEN 'PASS' ELSE 'FAIL linemerge' END;
SELECT CASE WHEN st_numgeometries(st_collectionextract(st_geomfromtext('GEOMETRYCOLLECTION(POLYGON((0 0,1 0,1 1,0 0)),POINT(2 2))'),3)) = 1 THEN 'PASS' ELSE 'FAIL collectionextract' END;
SELECT CASE WHEN st_numgeometries(st_forcecollection(st_point(1,1))) = 1 THEN 'PASS' ELSE 'FAIL forcecollection' END;
SELECT CASE WHEN st_geometrytype(st_multi(st_point(1,1))) = 'ST_MultiPoint' THEN 'PASS' ELSE 'FAIL multi' END;
SELECT CASE WHEN round(st_maxdistance(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'),st_geomfromtext('POLYGON((4 4,5 4,5 5,4 5,4 4))')),4) = round(sqrt(50.0),4) THEN 'PASS' ELSE 'FAIL maxdistance' END;
SELECT CASE WHEN round(st_length(st_longestline(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'),st_geomfromtext('POLYGON((4 4,5 4,5 5,4 5,4 4))'))),4) = round(sqrt(50.0),4) THEN 'PASS' ELSE 'FAIL longestline' END;
SELECT CASE WHEN st_astext(st_shortestline(st_point(1,1),st_geomfromtext('LINESTRING(0 0,2 0)'))) = 'LINESTRING(1 1,1 0)' THEN 'PASS' ELSE 'FAIL shortestline' END;
SELECT CASE WHEN st_nrings(st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))')) = 2 THEN 'PASS' ELSE 'FAIL nrings' END;
SELECT CASE WHEN st_numinteriorring(st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))')) = 1 THEN 'PASS' ELSE 'FAIL numinteriorring' END;
SELECT CASE WHEN st_orderingequals(st_point(1,1), st_point(1,1)) = true THEN 'PASS' ELSE 'FAIL orderingequals' END;
SELECT CASE WHEN st_ispolygon(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))')) = true THEN 'PASS' ELSE 'FAIL ispolygon' END;
SELECT CASE WHEN st_ispoint(st_point(1,1)) = true THEN 'PASS' ELSE 'FAIL ispoint' END;
SELECT CASE WHEN st_islinestring(st_geomfromtext('LINESTRING(0 0,1 1)')) = true THEN 'PASS' ELSE 'FAIL islinestring' END;
SELECT CASE WHEN st_asewkb(st_point(1,2)) IS NOT NULL THEN 'PASS' ELSE 'FAIL asewkb' END;
SELECT CASE WHEN st_astext(st_geomfromewkb(st_asewkb(st_point(1,2)))) = 'POINT(1 2)' THEN 'PASS' ELSE 'FAIL geomfromewkb' END;
SELECT CASE WHEN length(st_ashexewkb(st_point(1,2))) = 42 THEN 'PASS' ELSE 'FAIL ashexewkb' END;
SELECT CASE WHEN abs(st_area(st_triangulatepolygon(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'))) - 4.0) < 0.0001 THEN 'PASS' ELSE 'FAIL triangulatepolygon' END;

-- === Set-returning table functions: ST_Dump family ===
SELECT CASE WHEN (SELECT count(*) FROM st_dump(st_geomfromtext('MULTIPOLYGON(((0 0,1 0,1 1,0 0)),((2 2,3 2,3 3,2 2)))'))) = 2 THEN 'PASS' ELSE 'FAIL dump' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dump(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'))) = 1 THEN 'PASS' ELSE 'FAIL dump_atomic' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dump(st_geomfromtext('GEOMETRYCOLLECTION(POINT(1 2),POINT(3 4))'))) = 2 THEN 'PASS' ELSE 'FAIL dump_path' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(st_geomfromtext('LINESTRING(0 0,1 1,2 2)'))) = 3 THEN 'PASS' ELSE 'FAIL dumppoints' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dumpsegments(st_geomfromtext('LINESTRING(0 0,1 1,2 2)'))) = 2 THEN 'PASS' ELSE 'FAIL dumpsegments' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'))) = 4 THEN 'PASS' ELSE 'FAIL dumppoints_polygon' END;

-- === Tier 1/1b round 2 (constructors, editing, measurements, validity) ===
SELECT CASE WHEN st_area(st_makeenvelope(0,0,4,4)) = 16.0 THEN 'PASS' ELSE 'FAIL makeenvelope' END;
SELECT CASE WHEN st_geometrytype(st_makepolygon(st_geomfromtext('LINESTRING(0 0,1 0,1 1,0 0)'))) = 'ST_Polygon' THEN 'PASS' ELSE 'FAIL makepolygon' END;
SELECT CASE WHEN st_astext(st_makepoint(3,4)) = 'POINT(3 4)' THEN 'PASS' ELSE 'FAIL makepoint' END;
SELECT CASE WHEN st_numpoints(st_removepoint(st_geomfromtext('LINESTRING(0 0,1 1,2 2)'),1)) = 2 THEN 'PASS' ELSE 'FAIL removepoint' END;
SELECT CASE WHEN st_numpoints(st_addpoint(st_geomfromtext('LINESTRING(0 0,1 1)'),st_point(2,2))) = 3 THEN 'PASS' ELSE 'FAIL addpoint' END;
SELECT CASE WHEN st_isvalid(st_simplifypreservetopology(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),0.1)) = true THEN 'PASS' ELSE 'FAIL simplifypreservetopology' END;
SELECT CASE WHEN st_minimumclearance(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') ) > 0 THEN 'PASS' ELSE 'FAIL minimumclearance' END;
SELECT CASE WHEN st_minimumclearanceline(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) IS NOT NULL THEN 'PASS' ELSE 'FAIL minimumclearanceline' END;
SELECT CASE WHEN st_minimumboundingcircle(st_geomfromtext('MULTIPOINT(0 0,2 0,1 1,1 -1)'),32) IS NOT NULL THEN 'PASS' ELSE 'FAIL minimumboundingcircle' END;
SELECT CASE WHEN st_numgeometries(st_generatepoints(st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),20)) = 20 THEN 'PASS' ELSE 'FAIL generatepoints' END;
SELECT CASE WHEN st_isvalidreason(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) = 'Valid Geometry' THEN 'PASS' ELSE 'FAIL isvalidreason' END;

-- === Aggregate: ST_MakeLine ===
SELECT CASE WHEN st_numpoints(st_makeline_agg(g)) = 3 THEN 'PASS' ELSE 'FAIL makeline_agg' END FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(1,1) UNION ALL SELECT st_point(2,2));
-- NOTE: aggregate ORDER BY (e.g. st_makeline_agg(g ORDER BY k)) currently
-- segfaults via the DuckDB C-API aggregate callback path. See ROADMAP M27
-- and tests/reference/m27_known_issues.sql for the tracked bug.

-- === Tier 1 remaining: ST_Snap, ST_Subdivide, ST_Node, ST_Intersection agg ===
SELECT CASE WHEN st_astext(st_snap(st_geomfromtext('POINT(0.001 0)'), st_geomfromtext('POINT(0 0)'), 0.01)) = 'POINT(0 0)' THEN 'PASS' ELSE 'FAIL snap' END;
SELECT CASE WHEN st_numgeometries(st_subdivide(st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3,4 4,5 5)'),2)) >= 2 THEN 'PASS' ELSE 'FAIL subdivide' END;
SELECT CASE WHEN st_geometrytype(st_node(st_geomfromtext('MULTILINESTRING((0 0,2 2),(2 0,0 2))'))) = 'ST_MultiLineString' THEN 'PASS' ELSE 'FAIL node' END;
-- GEOS-backed topology (Phase 2): PostGIS-grade planar operations
SELECT CASE WHEN st_numgeometries(st_node(st_geomfromtext('MULTILINESTRING((0 0,4 4),(0 4,4 0))'))) = 4 THEN 'PASS' ELSE 'FAIL geos-node-crossing' END;
SELECT CASE WHEN st_numgeometries(st_polygonize(st_geomfromtext('LINESTRING(0 0,4 0,4 4,0 4,0 0)'))) = 1 THEN 'PASS' ELSE 'FAIL geos-polygonize' END;
SELECT CASE WHEN abs(st_area(st_buildarea(st_geomfromtext('MULTILINESTRING((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'))) - 15.0) < 1e-6 THEN 'PASS' ELSE 'FAIL geos-buildarea' END;
-- The 3x3 Voronoi grid that defeated the earlier angle-sort prototype (now GEOS)
SELECT CASE WHEN st_numgeometries(st_voronoipolygons(st_geomfromtext('MULTIPOINT((0 0),(1 0),(2 0),(0 1),(1 1),(2 1),(0 2),(1 2),(2 2))'))) = 9 THEN 'PASS' ELSE 'FAIL geos-voronoi-grid' END;
-- Spheroid geodesics (Karney / GeographicLib, WGS84)
SELECT CASE WHEN abs(st_distancespheroid(st_point(-0.1278, 51.5074), st_point(2.3522, 48.8566)) - 343924.0) < 50.0 THEN 'PASS' ELSE 'FAIL spheroid-distance' END;
SELECT CASE WHEN abs(st_lengthspheroid(st_geomfromtext('LINESTRING(0 0,1 0)')) - 111319.0) < 1.0 THEN 'PASS' ELSE 'FAIL spheroid-length' END;
SELECT CASE WHEN st_areaspheroid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) > 1.2e10 THEN 'PASS' ELSE 'FAIL spheroid-area' END;
SELECT CASE WHEN st_dwithinspheroid(st_point(-0.1278, 51.5074), st_point(2.3522, 48.8566), 400000.0) = true THEN 'PASS' ELSE 'FAIL spheroid-dwithin' END;
SELECT CASE WHEN st_area(st_intersection_agg(g)) = 16.0 THEN 'PASS' ELSE 'FAIL intersection_agg' END FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') g UNION ALL SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));
