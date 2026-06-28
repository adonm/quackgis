-- postgis_processing.sql -- curated processing/validation function tests.
-- MakeValid, ConcaveHull, OrientedEnvelope, FrechetDistance,
-- MinimumBoundingCircle, ShortestLine, LongestLine, MaxDistance, ClosestPoint,
-- Intersection, Difference, SymDifference.
.mode list

SELECT CASE WHEN st_isvalid(st_makevalid(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'))) = true THEN 'PASS makevalid bowtie' ELSE 'FAIL makevalid bowtie' END;
SELECT CASE WHEN st_isvalid(st_makevalid(st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'))) = true THEN 'PASS makevalid already' ELSE 'FAIL makevalid already' END;
SELECT CASE WHEN st_area(st_concavehull(st_geomfromtext('MULTIPOINT((0 0),(10 0),(10 10),(0 10),(5 5))'), 0.5)) > 0 THEN 'PASS concavehull points' ELSE 'FAIL concavehull points' END;
SELECT CASE WHEN st_concavehull(st_geomfromtext('MULTIPOINT((0 0),(5 0),(10 0))'), 0.5) IS NOT NULL THEN 'PASS concavehull collinear' ELSE 'FAIL concavehull collinear' END;
SELECT CASE WHEN st_area(st_orientedenvelope(st_geomfromtext('MULTIPOINT((0 0),(10 0),(10 10),(0 10),(5 5))'))) > 0 THEN 'PASS oriented_envelope' ELSE 'FAIL oriented_envelope' END;
SELECT CASE WHEN st_frechetdistance(st_geomfromtext('LINESTRING(0 0,1 1)'), st_geomfromtext('LINESTRING(0 0,1 1)')) = 0.0 THEN 'PASS frechet identical' ELSE 'FAIL frechet identical' END;
SELECT CASE WHEN st_frechetdistance(st_geomfromtext('LINESTRING(0 0,1 0)'), st_geomfromtext('LINESTRING(0 0,0 1)')) > 0.0 THEN 'PASS frechet different' ELSE 'FAIL frechet different' END;
SELECT CASE WHEN st_area(st_minimumboundingcircle(st_geomfromtext('MULTIPOINT((0 0),(1 0),(0 1),(1 1))'), 8)) > 0 THEN 'PASS minbound_circle' ELSE 'FAIL minbound_circle' END;
SELECT CASE WHEN st_numpoints(st_shortestline(st_point(0,0), st_point(5,5))) = 2 THEN 'PASS shortestline' ELSE 'FAIL shortestline' END;
SELECT CASE WHEN st_numpoints(st_longestline(st_point(0,0), st_point(5,5))) = 2 THEN 'PASS longestline' ELSE 'FAIL longestline' END;
SELECT CASE WHEN st_maxdistance(st_point(0,0), st_point(3,4)) = 5.0 THEN 'PASS maxdistance' ELSE 'FAIL maxdistance' END;
SELECT CASE WHEN st_maxdistance(st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'), st_geomfromtext('POLYGON((20 0,30 0,30 10,20 10,20 0))')) > 30.0 THEN 'PASS maxdistance polygons' ELSE 'FAIL maxdistance polygons' END;
SELECT CASE WHEN st_numpoints(st_closestpoint(st_geomfromtext('LINESTRING(0 0,10 0)'), st_point(5, 5))) = 1 THEN 'PASS closestpoint' ELSE 'FAIL closestpoint' END;
SELECT CASE WHEN abs(st_area(st_intersection(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'), st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 4.0) < 1e-12 THEN 'PASS intersection' ELSE 'FAIL intersection' END;
SELECT CASE WHEN abs(st_area(st_difference(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'), st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 12.0) < 1e-6 THEN 'PASS difference' ELSE 'FAIL difference' END;
SELECT CASE WHEN st_area(st_symdifference(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'), st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) > 0 THEN 'PASS symdifference' ELSE 'FAIL symdifference' END;
