.mode list
-- Month 23b: ST_AsMVT (Mapbox Vector Tile) encoder.
--
-- ST_AsMVT(geom) encodes a single geometry as a protobuf MVT tile (one layer,
-- one feature). Coordinates must already be in tile-local integer space (0..4096).
-- Hex-encoded VARCHAR output.
--
-- MVT geometry commands: MoveTo(1), LineTo(2), ClosePath(7).
-- Coordinate parameters are zigzag-varint deltas from the previous vertex.

-- Point(10 20): Tile{Layer{ver=2,name="geometry",Feature{type=POINT,
--   geom=[MoveTo1, zz(10)=20, zz(20)=40]}, extent=4096}}
SELECT CASE WHEN st_asmvt(st_geomfromtext('POINT(10 20)')) =
    '1a1878020a0867656f6d65747279120718012203091428288020'
THEN 'PASS m23b_mvt_point' ELSE 'FAIL m23b_mvt_point' END;

-- Point(0 0): MoveTo1, zz(0)=0, zz(0)=0
SELECT CASE WHEN st_asmvt(st_geomfromtext('POINT(0 0)')) IS NOT NULL
THEN 'PASS m23b_mvt_point_origin' ELSE 'FAIL m23b_mvt_point_origin' END;

-- LineString(0 0, 100 100, 200 50)
SELECT CASE WHEN st_asmvt(st_geomfromtext('LINESTRING(0 0,100 100,200 50)')) =
    '1a2078020a0867656f6d65747279120f1802220b09000012c801c801c80163288020'
THEN 'PASS m23b_mvt_linestring' ELSE 'FAIL m23b_mvt_linestring' END;

-- Polygon((0 0,0 100,100 100,100 0,0 0))
SELECT CASE WHEN st_asmvt(st_geomfromtext('POLYGON((0 0,0 100,100 100,100 0,0 0))')) =
    '1a2378020a0867656f6d6574727912121803220e0900001300c801c8010000c70139288020'
THEN 'PASS m23b_mvt_polygon' ELSE 'FAIL m23b_mvt_polygon' END;

-- MultiPoint — output should contain type=POINT
SELECT CASE WHEN st_asmvt(st_geomfromtext('MULTIPOINT((10 20),(30 40))')) IS NOT NULL
THEN 'PASS m23b_mvt_multipoint' ELSE 'FAIL m23b_mvt_multipoint' END;

-- MultiLineString
SELECT CASE WHEN st_asmvt(st_geomfromtext('MULTILINESTRING((0 0,10 10),(20 20,30 30))')) IS NOT NULL
THEN 'PASS m23b_mvt_multilinestring' ELSE 'FAIL m23b_mvt_multilinestring' END;

-- MultiPolygon
SELECT CASE WHEN st_asmvt(st_geomfromtext(
    'MULTIPOLYGON(((0 0,0 10,10 10,10 0,0 0)),((20 20,20 30,30 30,30 20,20 20)))')) IS NOT NULL
THEN 'PASS m23b_mvt_multipolygon' ELSE 'FAIL m23b_mvt_multipolygon' END;

-- NULL → NULL
SELECT CASE WHEN st_asmvt(NULL) IS NULL
THEN 'PASS m23b_mvt_null' ELSE 'FAIL m23b_mvt_null' END;

-- Output is valid protobuf: starts with field 3 tag (0x1a = layers field)
SELECT CASE WHEN st_asmvt(st_geomfromtext('POINT(1 2)')) LIKE '1a%'
THEN 'PASS m23b_mvt_protobuf_tag' ELSE 'FAIL m23b_mvt_protobuf_tag' END;

-- Output contains layer name "geometry"
SELECT CASE WHEN st_asmvt(st_geomfromtext('POINT(1 2)')) LIKE '%67656f6d65747279%'
THEN 'PASS m23b_mvt_layer_name' ELSE 'FAIL m23b_mvt_layer_name' END;
