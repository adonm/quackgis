-- SPDX-License-Identifier: Apache-2.0
-- Adversarial edge-case fixtures: degenerate, empty, Z-dimension, CRS-tagged,
-- and out-of-range inputs through BOTH the local st_* and literal sedona_st_*
-- paths. The goal is "no silent wrong geometry" — every undefined/degenerate
-- case must either agree between implementations or fail closed to NULL.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/edge_cases.sql
.bail off
.mode list

-- === Empty geometries: accessors must agree (local vs literal) ===
WITH empties(label, g) AS (
    SELECT * FROM (VALUES
        ('point empty',  st_geomfromtext('POINT EMPTY')),
        ('line empty',   st_geomfromtext('LINESTRING EMPTY')),
        ('poly empty',   st_geomfromtext('POLYGON EMPTY')),
        ('coll empty',   st_geomfromtext('GEOMETRYCOLLECTION EMPTY'))
    )
)
SELECT CASE WHEN (SELECT count(*) FROM empties WHERE
        st_isempty(g)    IS DISTINCT FROM sedona_st_isempty(g)
     OR st_isclosed(g)   IS DISTINCT FROM sedona_st_isclosed(g)) = 0
            THEN 'PASS empty-geom accessors agree'
            ELSE 'FAIL empty-geom accessor mismatch' END;

-- === All empty geometries must report isempty=true in both paths ===
WITH empties(g) AS (
    SELECT st_geomfromtext('POINT EMPTY')
    UNION ALL SELECT st_geomfromtext('LINESTRING EMPTY')
    UNION ALL SELECT st_geomfromtext('POLYGON EMPTY')
)
SELECT CASE WHEN (SELECT count(*) FROM empties WHERE
        st_isempty(g) = true AND sedona_st_isempty(g) = true) = 3
            THEN 'PASS isempty=true on all empties'
            ELSE 'FAIL isempty not true on empties' END;

-- === NULL propagation: NULL in → NULL out (fail-closed, no panic) ===
SELECT CASE WHEN st_dimension(NULL) IS NULL AND sedona_st_dimension(NULL) IS NULL
            THEN 'PASS NULL propagation' ELSE 'FAIL NULL not propagated' END;
SELECT CASE WHEN st_astext(CAST(NULL AS BLOB)) IS NULL AND sedona_st_astext(CAST(NULL AS BLOB)) IS NULL
            THEN 'PASS NULL astext' ELSE 'FAIL NULL astext not propagated' END;

-- === Z-dimension geometries: force2d strips Z, zmflag reads the dim flag ===
-- Local st_geomfromtext is 2D-only (drops Z), so build Z/M WKB via the literal
-- bridge constructors which preserve the dimension in the WKB byte stream.
WITH zpts(g) AS (
    SELECT sedona_st_pointz(1.0, 2.0, 3.0)
    UNION ALL SELECT sedona_st_pointm(1.0, 2.0, 3.0)
    UNION ALL SELECT sedona_st_pointzm(1.0, 2.0, 3.0, 4.0)
)
-- Local zmflag is 2D-only (returns 0); literal sedona_st_zmflag reads the
-- WKB dimension flag. Assert the literal reads the flag and literal force2d
-- collapses to a single 2D vertex.
SELECT CASE WHEN (SELECT count(*) FROM zpts WHERE
        sedona_st_zmflag(g) > 0
     AND st_numpoints(sedona_st_force2d(g)) = 1) = 3
            THEN 'PASS Z-dim force2d + zmflag'
            ELSE 'FAIL Z-dim handling' END;

-- === Large / negative coordinates: ordinate accessors agree ===
WITH big(g) AS (
    SELECT st_geomfromtext('POINT(-1e15 1e15)')
    UNION ALL SELECT st_geomfromtext('POINT(0.000001 0.000001)')
    UNION ALL SELECT st_geomfromtext('POINT(-180 -90)')
)
SELECT CASE WHEN (SELECT count(*) FROM big WHERE
        abs(st_x(g) - sedona_st_x(g)) > 1e-9
     OR abs(st_y(g) - sedona_st_y(g)) > 1e-9) = 0
            THEN 'PASS large/negative coords agree'
            ELSE 'FAIL coordinate mismatch on extremes' END;

-- === CRS-tagged EWKT: struct unwrapped to plain WKB, geometry preserved ===
SELECT CASE WHEN st_astext(sedona_st_geomfromewkt('SRID=4326;LINESTRING(0 0,1 1)')) = 'LINESTRING(0 0,1 1)'
            THEN 'PASS CRS-tagged EWKT unwrapped' ELSE 'FAIL CRS EWKT unwrap' END;

-- === Cocircular / collinear points: no crash, deterministic output ===
-- Three collinear points — Delaunay/Voronoi degenerate, but accessors must work.
WITH collinear(g) AS (
    SELECT st_geomfromtext('MULTIPOINT((0 0),(1 0),(2 0))')
)
SELECT CASE WHEN sedona_st_dimension(g) = 0 AND sedona_st_numgeometries(g) = 3
            THEN 'PASS collinear multipoint accessors'
            ELSE 'FAIL collinear handling' END FROM collinear;

-- === WKB constructor round-trip preserves geometry ===
WITH rt(wkb) AS (
    SELECT st_asbinary(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'))
)
SELECT CASE WHEN st_astext(sedona_st_geomfromwkb(wkb)) = 'POLYGON((0 0,1 0,1 1,0 0))'
            THEN 'PASS WKB round-trip' ELSE 'FAIL WKB round-trip' END FROM rt;

-- === Degenerate line (single repeated point): no crash ===
SELECT CASE WHEN sedona_st_numpoints(st_geomfromtext('LINESTRING(1 1,1 1)')) = 2
            THEN 'PASS degenerate line' ELSE 'FAIL degenerate line' END;
