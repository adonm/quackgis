-- SPDX-License-Identifier: Apache-2.0
--
-- QuackGIS function stubs, operators.
-- geometry type is registered by the quackgis_type C extension (CREATE EXTENSION).
-- Order: bridge → stubs → operators → PostGIS compat.

-- ══ Bridge table ═══════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS quackgis._bridge (id int) USING ducklake;
DO $$ BEGIN INSERT INTO quackgis._bridge VALUES (1); EXCEPTION WHEN OTHERS THEN NULL; END $$;

-- ══ Function stubs (define BEFORE casts/operators that reference them) ═════

CREATE OR REPLACE FUNCTION st_geomfromtext(wkt text)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_geomfromtext($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_geomfromtext(wkt text, srid int)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_geomfromtext($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_point(x double precision, y double precision)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_point($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_makepoint(x double precision, y double precision)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_point($1, $2) $$;
CREATE OR REPLACE FUNCTION st_makeenvelope(xmin double precision, ymin double precision, xmax double precision, ymax double precision, srid int DEFAULT 0)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_makeenvelope($1, $2, $3, $4) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_astext(g bytea)
RETURNS text LANGUAGE sql AS $$ SELECT st_astext($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_asewkt(g bytea)
RETURNS text LANGUAGE sql AS $$ SELECT st_asewkt($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_asbinary(g bytea)
RETURNS bytea LANGUAGE sql AS $$ SELECT st_asbinary($1)::bytea FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_asgeojson(g bytea)
RETURNS text LANGUAGE sql AS $$ SELECT st_asgeojson($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_area(g bytea)
RETURNS double precision LANGUAGE sql AS $$ SELECT st_area($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_length(g bytea)
RETURNS double precision LANGUAGE sql AS $$ SELECT st_length($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_x(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_x($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_y(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_y($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_srid(g bytea) RETURNS int LANGUAGE sql AS $$ SELECT st_srid($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_setsrid(g bytea, srid int)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_setsrid($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_numpoints(g bytea) RETURNS int LANGUAGE sql AS $$ SELECT st_numpoints($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_isempty(g bytea) RETURNS boolean LANGUAGE sql AS $$ SELECT st_isempty($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_isvalid(g bytea) RETURNS boolean LANGUAGE sql AS $$ SELECT st_isvalid($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_geometrytype(g bytea) RETURNS text LANGUAGE sql AS $$ SELECT st_geometrytype($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_dimension(g bytea) RETURNS int LANGUAGE sql AS $$ SELECT st_dimension($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_envelope(g bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_envelope($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_centroid(g bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_centroid($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_buffer(g bytea, r double precision)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_buffer($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_intersects(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_intersects($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_contains(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_contains($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_within(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_within($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_disjoint(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_disjoint($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_touches(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_touches($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_crosses(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_crosses($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_overlaps(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_overlaps($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_equals(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_equals($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_covers(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_covers($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_coveredby(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_coveredby($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_dwithin(g1 bytea, g2 bytea, d double precision)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_dwithin($1, $2, $3) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_relate(g1 bytea, g2 bytea)
RETURNS text LANGUAGE sql AS $$ SELECT st_relate($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_distance(g1 bytea, g2 bytea)
RETURNS double precision LANGUAGE sql AS $$ SELECT st_distance($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_distancesphere(g1 bytea, g2 bytea)
RETURNS double precision LANGUAGE sql AS $$ SELECT st_distancesphere($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_distancespheroid(g1 bytea, g2 bytea)
RETURNS double precision LANGUAGE sql AS $$ SELECT st_distancespheroid($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_intersection(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_intersection($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_union(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_union($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_difference(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_difference($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_symdifference(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_symdifference($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_transform(g bytea, srid int)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_transform($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_transform(g bytea, from_srid int, to_srid int)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_transform($1, $2, $3) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_simplify(g bytea, tol double precision)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_simplify($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_convexhull(g bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_convexhull($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_makevalid(g bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_makevalid($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_collect_scalar(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_collect_scalar($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_collect(g1 bytea, g2 bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_collect_scalar($1, $2) $$;
CREATE OR REPLACE FUNCTION st_multi(g bytea)
RETURNS geometry LANGUAGE sql AS $$ SELECT st_multi($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_bbox_intersects(g1 bytea, g2 bytea)
RETURNS boolean LANGUAGE sql AS $$ SELECT st_bbox_intersects($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_xmin(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_xmin($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_ymin(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_ymin($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_xmax(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_xmax($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_ymax(g bytea) RETURNS double precision LANGUAGE sql AS $$ SELECT st_ymax($1) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_quadkey(g bytea, zoom int)
RETURNS text LANGUAGE sql AS $$ SELECT st_quadkey($1, $2) FROM quackgis._bridge LIMIT 1 $$;
CREATE OR REPLACE FUNCTION st_hilbert(g bytea, bits int)
RETURNS bigint LANGUAGE sql AS $$ SELECT st_hilbert($1, $2) FROM quackgis._bridge LIMIT 1 $$;

-- ══ Note: text::geometry cast doesn't work with DOMAIN; use st_geomfromtext ═

-- ══ Operators on bytea (pg_ducklake won't cast to GEOMETRY; DOMAIN auto-converts) ══

CREATE OR REPLACE FUNCTION quackgis.geom_overlap(a bytea, b bytea)
RETURNS boolean LANGUAGE sql AS $$
    SELECT st_bbox_intersects($1, $2) FROM quackgis._bridge LIMIT 1 $$;

DROP OPERATOR IF EXISTS && (bytea, bytea);
DO $$ BEGIN CREATE OPERATOR && (
    LEFTARG = bytea, RIGHTARG = bytea,
    PROCEDURE = quackgis.geom_overlap, COMMUTATOR = &&);
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

CREATE OR REPLACE FUNCTION quackgis.geom_distance(a bytea, b bytea)
RETURNS double precision LANGUAGE sql AS $$
    SELECT st_distance($1, $2) FROM quackgis._bridge LIMIT 1 $$;

DROP OPERATOR IF EXISTS <-> (bytea, bytea);
DO $$ BEGIN CREATE OPERATOR <-> (
    LEFTARG = bytea, RIGHTARG = bytea,
    PROCEDURE = quackgis.geom_distance, COMMUTATOR = <->);
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DROP OPERATOR IF EXISTS <#> (bytea, bytea);
DO $$ BEGIN CREATE OPERATOR <#> (
    LEFTARG = bytea, RIGHTARG = bytea,
    PROCEDURE = quackgis.geom_distance, COMMUTATOR = <#>);
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

-- ══ PostGIS compat (geometry_columns, spatial_ref_sys, postgis_version 
--    are provided by the quackgis_type C extension) ═════════════════════════

CREATE OR REPLACE FUNCTION quackgis.rewrite_sql(input_sql text)
RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        RETURN (SELECT sedonadb_rewrite_postgis(input_sql) FROM quackgis._bridge LIMIT 1);
    EXCEPTION WHEN OTHERS THEN
        RETURN input_sql;
    END;
END $$;

CREATE OR REPLACE FUNCTION quackgis.compat_check()
RETURNS TABLE(feature text, status text, detail text)
LANGUAGE plpgsql AS $$
BEGIN
    RETURN QUERY SELECT 'geometry type', 'OK', 'DOMAIN over bytea'
    WHERE EXISTS (SELECT 1 FROM pg_type WHERE typname = 'geometry');
    RETURN QUERY SELECT '&&', 'OK', 'bbox overlap'
    WHERE EXISTS (SELECT 1 FROM pg_operator WHERE oprname = '&&' AND oprleft = 'geometry'::regtype);
    RETURN QUERY SELECT '<->', 'OK', 'KNN distance'
    WHERE EXISTS (SELECT 1 FROM pg_operator WHERE oprname = '<->' AND oprleft = 'geometry'::regtype);
    RETURN QUERY SELECT 'pg_ducklake', 'OK', 'DuckLake table AM'
    WHERE EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_ducklake');
    RETURN QUERY SELECT 'postgis_version', 'OK', 'compat'
    WHERE EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'postgis_version');
END $$;
