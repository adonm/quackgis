.mode list
-- Milestone 7: compatibility evidence and namespace closure.
-- Covers: literal routing fixes (st_linesubstring, st_geomfromwkt), new
-- namespace gaps (ST_SetPoint, ST_IsValidDetail), and routing parity.

-- =====================================================================
-- 1. st_linesubstring parity: st_* == sedona_st_* (was local, now routed)
-- =====================================================================

SELECT CASE WHEN st_equals(
    st_linesubstring(st_geomfromtext('LINESTRING(0 0, 10 0)'), 0.25, 0.75),
    sedona_st_linesubstring(sedona_st_geomfromewkt('LINESTRING(0 0, 10 0)'), 0.25, 0.75)
) THEN 'PASS linesubstring_parity' ELSE 'FAIL linesubstring_parity' END;

-- Functional correctness: known substring of LINESTRING(0 0, 10 0) at [0.25, 0.75].
SELECT CASE WHEN st_astext(st_linesubstring(
    st_geomfromtext('LINESTRING(0 0, 10 0)'), 0.25, 0.75
)) LIKE 'LINESTRING(2.5 0,7.5 0)'
THEN 'PASS linesubstring_functional' ELSE 'FAIL linesubstring_functional' END;

-- NULL propagation
SELECT CASE WHEN st_linesubstring(NULL, 0.0, 1.0) IS NULL
THEN 'PASS linesubstring_null' ELSE 'FAIL linesubstring_null' END;

-- =====================================================================
-- 2. st_geomfromwkt alias (SedonaDB naming for WKT constructor)
-- =====================================================================

SELECT CASE WHEN st_equals(
    st_geomfromwkt('POINT(1 2)'),
    st_geomfromtext('POINT(1 2)')
) THEN 'PASS geomfromwkt_alias' ELSE 'FAIL geomfromwkt_alias' END;

-- Parity with literal kernel
SELECT CASE WHEN st_equals(
    st_geomfromwkt('POLYGON((0 0,1 0,1 1,0 0))'),
    sedona_st_geomfromwkt('POLYGON((0 0,1 0,1 1,0 0))')
) THEN 'PASS geomfromwkt_parity' ELSE 'FAIL geomfromwkt_parity' END;

-- NULL propagation
SELECT CASE WHEN st_geomfromwkt(NULL) IS NULL
THEN 'PASS geomfromwkt_null' ELSE 'FAIL geomfromwkt_null' END;

-- =====================================================================
-- 3. ST_SetPoint functional tests
-- =====================================================================

-- Replace 0th point
SELECT CASE WHEN st_astext(st_setpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)'),
    0,
    st_geomfromtext('POINT(5 5)')
)) LIKE 'LINESTRING(5 5,1 1,2 2)'
THEN 'PASS setpoint_index0' ELSE 'FAIL setpoint_index0' END;

-- Replace last point
SELECT CASE WHEN st_astext(st_setpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)'),
    2,
    st_geomfromtext('POINT(9 9)')
)) LIKE 'LINESTRING(0 0,1 1,9 9)'
THEN 'PASS setpoint_last' ELSE 'FAIL setpoint_last' END;

-- Replace middle point
SELECT CASE WHEN st_astext(st_setpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1, 2 2, 3 3)'),
    1,
    st_geomfromtext('POINT(7 7)')
)) LIKE 'LINESTRING(0 0,7 7,2 2,3 3)'
THEN 'PASS setpoint_middle' ELSE 'FAIL setpoint_middle' END;

-- Out of range → NULL
SELECT CASE WHEN st_setpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1)'),
    5,
    st_geomfromtext('POINT(9 9)')
) IS NULL
THEN 'PASS setpoint_out_of_range' ELSE 'FAIL setpoint_out_of_range' END;

-- Non-linestring input → NULL
SELECT CASE WHEN st_setpoint(
    st_geomfromtext('POINT(0 0)'),
    0,
    st_geomfromtext('POINT(1 1)')
) IS NULL
THEN 'PASS setpoint_non_linestring' ELSE 'FAIL setpoint_non_linestring' END;

-- NULL propagation
SELECT CASE WHEN st_setpoint(NULL, 0, st_geomfromtext('POINT(1 1)')) IS NULL
THEN 'PASS setpoint_null' ELSE 'FAIL setpoint_null' END;

-- =====================================================================
-- 4. ST_IsValidDetail table function
-- =====================================================================

-- Valid geometry → (true, 'Valid Geometry', ...)
SELECT CASE WHEN valid = true AND reason = 'Valid Geometry'
THEN 'PASS isvaliddetail_valid' ELSE 'FAIL isvaliddetail_valid' END
FROM st_isvaliddetail(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'));

-- Invalid bowtie polygon → (false, reason, ...)
SELECT CASE WHEN valid = false AND length(reason) > 0
THEN 'PASS isvaliddetail_invalid' ELSE 'FAIL isvaliddetail_invalid' END
FROM st_isvaliddetail(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'));

-- Valid point → (true, 'Valid Geometry', ...)
SELECT CASE WHEN valid = true
THEN 'PASS isvaliddetail_point' ELSE 'FAIL isvaliddetail_point' END
FROM st_isvaliddetail(st_geomfromtext('POINT(1 2)'));

-- Returns exactly one row
SELECT CASE WHEN count(*) = 1
THEN 'PASS isvaliddetail_one_row' ELSE 'FAIL isvaliddetail_one_row' END
FROM st_isvaliddetail(st_geomfromtext('LINESTRING(0 0, 1 1)'));

-- =====================================================================
-- 5. Routing parity batch: verify routed st_* == sedona_st_* on edge cases
-- =====================================================================

-- Empty geometry parity
SELECT CASE WHEN st_equals(
    st_envelope(st_geomfromtext('GEOMETRYCOLLECTION EMPTY')),
    sedona_st_envelope(sedona_st_geomfromewkt('GEOMETRYCOLLECTION EMPTY'))
) THEN 'PASS parity_envelope_empty' ELSE 'FAIL parity_envelope_empty' END;

-- Point with Z (use bridge constructor on both sides — local WKT is 2D-only,
-- documented delta. Tests that routed st_z == sedona_st_z on Z-enabled WKB.)
SELECT CASE WHEN st_z(sedona_st_geomfromewkt('POINT Z(1 2 3)')) =
                 sedona_st_z(sedona_st_geomfromewkt('POINT Z(1 2 3)'))
THEN 'PASS parity_z_ordinate' ELSE 'FAIL parity_z_ordinate' END;

-- SRID accessor: delta CLOSED — public st_setsrid writes an EWKB SRID tag on
-- the blob and st_srid reads it back (PostGIS semantics). The literal
-- sedona_st_* namespace still models SRID at the type level (read-back 0).
SELECT CASE WHEN
    st_srid(st_setsrid(st_geomfromtext('POINT(1 2)'), 4326)) = 4326
    AND
    sedona_st_srid(sedona_st_setsrid(sedona_st_geomfromewkt('POINT(1 2)'), 4326)) = 0
THEN 'PASS parity_srid_closed' ELSE 'FAIL parity_srid_closed' END;

-- Force2D parity
SELECT CASE WHEN st_equals(
    st_force2d(st_geomfromtext('POINT Z(1 2 3)')),
    sedona_st_force2d(sedona_st_geomfromewkt('POINT Z(1 2 3)'))
) THEN 'PASS parity_force2d' ELSE 'FAIL parity_force2d' END;

-- NumPoints / NumGeometries parity on collection
SELECT CASE WHEN
    st_numpoints(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)')) =
    sedona_st_numpoints(sedona_st_geomfromewkt('LINESTRING(0 0, 1 1, 2 2)'))
    AND
    st_numgeometries(st_geomfromtext('MULTIPOINT(0 0, 1 1)')) =
    sedona_st_numgeometries(sedona_st_geomfromewkt('MULTIPOINT(0 0, 1 1)'))
THEN 'PASS parity_numpoints' ELSE 'FAIL parity_numpoints' END;

-- AsText / AsBinary parity
SELECT CASE WHEN
    st_astext(st_geomfromtext('POINT(1 2)')) =
    sedona_st_astext(sedona_st_geomfromewkt('POINT(1 2)'))
    AND
    st_asbinary(st_geomfromtext('POINT(1 2)')) =
    sedona_st_asbinary(sedona_st_geomfromewkt('POINT(1 2)'))
THEN 'PASS parity_astext_asbinary' ELSE 'FAIL parity_astext_asbinary' END;

-- Translate / Scale / Rotate parity
SELECT CASE WHEN
    st_equals(
        st_translate(st_geomfromtext('POINT(1 1)'), 2.0, 3.0),
        sedona_st_translate(sedona_st_geomfromewkt('POINT(1 1)'), 2.0, 3.0)
    )
    AND
    st_equals(
        st_scale(st_geomfromtext('POINT(2 3)'), 2.0, 0.5),
        sedona_st_scale(sedona_st_geomfromewkt('POINT(2 3)'), 2.0, 0.5)
    )
    AND
    st_equals(
        st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966),
        sedona_st_rotate(sedona_st_geomfromewkt('POINT(1 0)'), 1.5707963267948966)
    )
THEN 'PASS parity_transforms' ELSE 'FAIL parity_transforms' END;

-- Affine parity
SELECT CASE WHEN st_equals(
    st_affine(st_geomfromtext('POINT(1 2)'), 1.0, 0.0, 0.0, 1.0, 10.0, 20.0),
    sedona_st_affine(sedona_st_geomfromewkt('POINT(1 2)'), 1.0, 0.0, 0.0, 1.0, 10.0, 20.0)
) THEN 'PASS parity_affine' ELSE 'FAIL parity_affine' END;

-- StartPoint / EndPoint parity
SELECT CASE WHEN
    st_equals(
        st_startpoint(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)')),
        sedona_st_startpoint(sedona_st_geomfromewkt('LINESTRING(0 0, 1 1, 2 2)'))
    )
    AND
    st_equals(
        st_endpoint(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)')),
        sedona_st_endpoint(sedona_st_geomfromewkt('LINESTRING(0 0, 1 1, 2 2)'))
    )
THEN 'PASS parity_endpoints' ELSE 'FAIL parity_endpoints' END;

-- IsEmpty / IsClosed parity
SELECT CASE WHEN
    st_isempty(st_geomfromtext('POINT EMPTY')) =
    sedona_st_isempty(sedona_st_geomfromewkt('POINT EMPTY'))
THEN 'PASS parity_isempty' ELSE 'FAIL parity_isempty' END;

-- FlipCoordinates parity
SELECT CASE WHEN st_equals(
    st_flipcoordinates(st_geomfromtext('POINT(1 2)')),
    sedona_st_flipcoordinates(sedona_st_geomfromewkt('POINT(1 2)'))
) THEN 'PASS parity_flip' ELSE 'FAIL parity_flip' END;

-- Reverse parity
SELECT CASE WHEN st_equals(
    st_reverse(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)')),
    sedona_st_reverse(sedona_st_geomfromewkt('LINESTRING(0 0, 1 1, 2 2)'))
) THEN 'PASS parity_reverse' ELSE 'FAIL parity_reverse' END;

-- Segmentize parity
SELECT CASE WHEN st_equals(
    st_segmentize(st_geomfromtext('LINESTRING(0 0, 10 0)'), 3.0),
    sedona_st_segmentize(sedona_st_geomfromewkt('LINESTRING(0 0, 10 0)'), 3.0)
) THEN 'PASS parity_segmentize' ELSE 'FAIL parity_segmentize' END;

-- Point / MakePoint constructor parity
SELECT CASE WHEN
    st_equals(st_point(1.0, 2.0), sedona_st_point(1.0, 2.0))
    AND
    st_equals(st_makepoint(3.0, 4.0), sedona_st_point(3.0, 4.0))
THEN 'PASS parity_point' ELSE 'FAIL parity_point' END;

-- Azimuth parity
SELECT CASE WHEN
    st_azimuth(st_point(0.0, 0.0), st_point(1.0, 0.0)) =
    sedona_st_azimuth(sedona_st_point(0.0, 0.0), sedona_st_point(1.0, 0.0))
THEN 'PASS parity_azimuth' ELSE 'FAIL parity_azimuth' END;

-- Typed WKT constructor parity
SELECT CASE WHEN
    st_equals(
        st_linefromtext('LINESTRING(0 0, 1 1)'),
        sedona_st_linefromtext('LINESTRING(0 0, 1 1)')
    )
    AND
    st_equals(
        st_pointfromtext('POINT(1 2)'),
        sedona_st_pointfromtext('POINT(1 2)')
    )
    AND
    st_equals(
        st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 0))'),
        sedona_st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 0))')
    )
THEN 'PASS parity_typed_wkt' ELSE 'FAIL parity_typed_wkt' END;
