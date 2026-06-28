.mode list
-- Port family: validity and make-valid
-- Tests that PostGIS validity SQL ports directly.

-- PG: SELECT ST_IsValid(ST_GeomFromText('POLYGON((0 0,1 0,1 1,0 1,0 0))'));
-- Expected: t
SELECT CASE WHEN st_isvalid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) = true
THEN 'PASS validity_valid_polygon' ELSE 'FAIL validity_valid_polygon' END;

-- PG: SELECT ST_IsValid(ST_GeomFromText('POLYGON((0 0,1 1,1 0,0 1,0 0))'));
-- Expected: f  (bowtie self-intersection)
SELECT CASE WHEN st_isvalid(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))')) = false
THEN 'PASS validity_bowtie' ELSE 'FAIL validity_bowtie' END;

-- PG: SELECT ST_IsValidReason(ST_GeomFromText('POLYGON((0 0,1 0,1 1,0 1,0 0))'));
-- Expected: Valid Geometry
SELECT CASE WHEN st_isvalidreason(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) = 'Valid Geometry'
THEN 'PASS validity_reason_valid' ELSE 'FAIL validity_reason_valid' END;

-- PG: SELECT ST_IsValidReason(ST_GeomFromText('POLYGON((0 0,1 1,1 0,0 1,0 0))'));
-- Expected: <non-empty error string>
SELECT CASE WHEN length(st_isvalidreason(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'))) > 0
              AND st_isvalidreason(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))')) != 'Valid Geometry'
THEN 'PASS validity_reason_invalid' ELSE 'FAIL validity_reason_invalid' END;

-- PG: SELECT ST_IsValid(ST_MakeValid(ST_GeomFromText('POLYGON((0 0,1 1,1 0,0 1,0 0))')));
-- Expected: t  (GEOS make_valid fixes the bowtie)
SELECT CASE WHEN st_isvalid(st_makevalid(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'))) = true
THEN 'PASS validity_makevalid' ELSE 'FAIL validity_makevalid' END;

-- PG: SELECT valid FROM ST_IsValidDetail(ST_GeomFromText('POLYGON((0 0,1 0,1 1,0 1,0 0))'));
-- Expected: t
SELECT CASE WHEN valid = true
THEN 'PASS validity_detail_valid' ELSE 'FAIL validity_detail_valid' END
FROM st_isvaliddetail(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'));

-- PG: SELECT valid FROM ST_IsValidDetail(ST_GeomFromText('POLYGON((0 0,1 1,1 0,0 1,0 0))'));
-- Expected: f
SELECT CASE WHEN valid = false
THEN 'PASS validity_detail_invalid' ELSE 'FAIL validity_detail_invalid' END
FROM st_isvaliddetail(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'));
