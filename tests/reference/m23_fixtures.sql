.mode list
-- Month 23: ST_AsKML and ST_AsTWKB output encoders.
--
-- ST_AsKML: KML 2.2 geometry serialization (XML text).
-- ST_AsTWKB: Tiny WKB compact binary (hex-encoded VARCHAR, precision=0).

-- === ST_AsKML ===

-- Point
SELECT CASE WHEN st_askml(st_geomfromtext('POINT(1 2)')) =
    '<Point><coordinates>1,2</coordinates></Point>'
THEN 'PASS m23_kml_point' ELSE 'FAIL m23_kml_point' END;

-- LineString
SELECT CASE WHEN st_askml(st_geomfromtext('LINESTRING(0 0,1 1,2 2)')) =
    '<LineString><coordinates>0,0 1,1 2,2</coordinates></LineString>'
THEN 'PASS m23_kml_linestring' ELSE 'FAIL m23_kml_linestring' END;

-- Polygon with hole
SELECT CASE WHEN st_askml(st_geomfromtext(
    'POLYGON((0 0,0 4,4 4,4 0,0 0),(1 1,1 2,2 2,2 1,1 1))')) LIKE
    '%<Polygon><outerBoundaryIs>%<innerBoundaryIs>%</Polygon>%'
THEN 'PASS m23_kml_polygon_hole' ELSE 'FAIL m23_kml_polygon_hole' END;

-- MultiPoint
SELECT CASE WHEN st_askml(st_geomfromtext('MULTIPOINT((1 2),(3 4))')) LIKE
    '%<MultiGeometry><Point><coordinates>1,2</coordinates></Point><Point><coordinates>3,4</coordinates></Point></MultiGeometry>%'
THEN 'PASS m23_kml_multipoint' ELSE 'FAIL m23_kml_multipoint' END;

-- MultiPolygon
SELECT CASE WHEN st_askml(st_geomfromtext(
    'MULTIPOLYGON(((0 0,0 1,1 1,1 0,0 0)),((2 2,2 3,3 3,3 2,2 2)))')) LIKE
    '%<MultiGeometry><Polygon>%</Polygon><Polygon>%</Polygon></MultiGeometry>%'
THEN 'PASS m23_kml_multipolygon' ELSE 'FAIL m23_kml_multipolygon' END;

-- NULL
SELECT CASE WHEN st_askml(NULL) IS NULL
THEN 'PASS m23_kml_null' ELSE 'FAIL m23_kml_null' END;

-- === ST_AsTWKB ===

-- Point(1 2): type=01, x=zigzag(1)=02, y=zigzag(2)=04 → 010204
SELECT CASE WHEN st_astwkb(st_geomfromtext('POINT(1 2)')) = '010204'
THEN 'PASS m23_twkb_point' ELSE 'FAIL m23_twkb_point' END;

-- Point(-1 -2): type=01, x=zigzag(-1)=01, y=zigzag(-2)=03 → 010103
SELECT CASE WHEN st_astwkb(st_geomfromtext('POINT(-1 -2)')) = '010103'
THEN 'PASS m23_twkb_negative' ELSE 'FAIL m23_twkb_negative' END;

-- LineString(0 0,1 1): type=02, npts=02, (0,0)=0000, delta(1,1)=0202 → 020200000202
SELECT CASE WHEN st_astwkb(st_geomfromtext('LINESTRING(0 0,1 1)')) = '020200000202'
THEN 'PASS m23_twkb_linestring' ELSE 'FAIL m23_twkb_linestring' END;

-- Polygon((0 0,0 4,4 4,4 0,0 0)): verified manually
SELECT CASE WHEN st_astwkb(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))')) =
    '03010500000008080000070700'
THEN 'PASS m23_twkb_polygon' ELSE 'FAIL m23_twkb_polygon' END;

-- MultiPoint((1 2),(3 4)): type=04, n=02, (1,2)=0204, delta(2,2)=0404 → 040202040404
SELECT CASE WHEN st_astwkb(st_geomfromtext('MULTIPOINT((1 2),(3 4))')) =
    '040202040404'
THEN 'PASS m23_twkb_multipoint' ELSE 'FAIL m23_twkb_multipoint' END;

-- NULL
SELECT CASE WHEN st_astwkb(NULL) IS NULL
THEN 'PASS m23_twkb_null' ELSE 'FAIL m23_twkb_null' END;

-- Empty geometry → type byte only with 0 points
SELECT CASE WHEN st_astwkb(st_geomfromtext('POINT EMPTY')) IS NOT NULL
THEN 'PASS m23_twkb_empty_notnull' ELSE 'FAIL m23_twkb_empty_notnull' END;
