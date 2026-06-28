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
SELECT CASE WHEN st_numgeometries(st_collect(g)) = 3 THEN 'PASS' ELSE 'FAIL collect' FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(1,1) UNION ALL SELECT st_point(2,2));
SELECT CASE WHEN st_astext(st_envelope_agg(g)) = 'POLYGON((0 0,5 0,5 8,0 8,0 0))' THEN 'PASS' ELSE 'FAIL envelope_agg' FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(5,2) UNION ALL SELECT st_point(2,8));
SELECT CASE WHEN st_area(st_union_agg(g)) = 2.0 THEN 'PASS' ELSE 'FAIL union_agg' FROM (SELECT st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))') g UNION ALL SELECT st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))'));

-- === Raster ===
SELECT CASE WHEN count = 16 THEN 'PASS' ELSE 'FAIL raster_stats' FROM st_raster_stats('/var/home/adonm/dev/duckdb_sedona/build/raster/test.tif', 1);

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
