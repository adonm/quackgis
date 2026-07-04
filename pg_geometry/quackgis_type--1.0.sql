-- quackgis_type--1.0.sql
-- PostgreSQL geometry type for QuackGIS.
-- Minimal: type + I/O functions + casts only.
-- Catalog views (geometry_columns, spatial_ref_sys) created in init scripts.

-- ── Type functions ─────────────────────────────────────────────────────────

CREATE FUNCTION geometry_in(cstring) RETURNS geometry
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

CREATE FUNCTION geometry_out(geometry) RETURNS cstring
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

CREATE FUNCTION geometry_recv(internal) RETURNS geometry
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

CREATE FUNCTION geometry_send(geometry) RETURNS bytea
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

CREATE FUNCTION geometry_typmod_in(cstring[]) RETURNS integer
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

CREATE FUNCTION geometry_typmod_out(integer) RETURNS cstring
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;

-- ── The geometry type ──────────────────────────────────────────────────────

CREATE TYPE geometry (
    INTERNALLENGTH = VARIABLE,
    INPUT = geometry_in,
    OUTPUT = geometry_out,
    RECEIVE = geometry_recv,
    SEND = geometry_send,
    TYPMOD_IN = geometry_typmod_in,
    TYPMOD_OUT = geometry_typmod_out,
    ALIGNMENT = double,
    STORAGE = EXTENDED,
    CATEGORY = 'U'
);

-- ── Casts ──────────────────────────────────────────────────────────────────

-- text → geometry (WKT parse via GEOS)
CREATE FUNCTION text_geometry(text) RETURNS geometry
    AS 'MODULE_PATHNAME', 'geometry_in' LANGUAGE C IMMUTABLE STRICT;
CREATE CAST (text AS geometry) WITH FUNCTION text_geometry(text) AS IMPLICIT;

-- geometry → text (WKT output via GEOS)
CREATE FUNCTION geometry_text(geometry) RETURNS text
    AS 'MODULE_PATHNAME', 'geometry_to_text' LANGUAGE C IMMUTABLE STRICT;
CREATE CAST (geometry AS text) WITH FUNCTION geometry_text(geometry) AS IMPLICIT;

-- geometry ↔ bytea (same internal format, C identity function)
CREATE FUNCTION geometry_to_bytea(geometry) RETURNS bytea
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;
CREATE CAST (geometry AS bytea) WITH FUNCTION geometry_to_bytea(geometry) AS IMPLICIT;

CREATE FUNCTION bytea_to_geometry(bytea) RETURNS geometry
    AS 'MODULE_PATHNAME' LANGUAGE C IMMUTABLE STRICT;
CREATE CAST (bytea AS geometry) WITH FUNCTION bytea_to_geometry(bytea) AS IMPLICIT;
