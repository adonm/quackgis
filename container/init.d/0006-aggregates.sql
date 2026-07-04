-- SPDX-License-Identifier: Apache-2.0
--
-- QuackGIS spatial aggregate stubs (M8).
--
-- PostgreSQL needs aggregates registered at the catalog level to parse
-- `SELECT st_union_agg(geom) FROM table GROUP BY ...`. These stubs collect
-- bytea values into an array, then delegate the final computation to
-- DuckDB/sedonadb via the bridge table pattern.
--
-- When a query touches a DuckLake table, pg_ducklake's planner hook routes
-- the entire query to DuckDB, where sedonadb's native aggregate runs.
-- The PG aggregate is the fallback for non-DuckLake contexts.

-- ── State type: array of bytea ──────────────────────────────────────────────

-- ── st_union_agg: cascaded polygonal union ──────────────────────────────────

CREATE OR REPLACE FUNCTION quackgis._union_state(state bytea[], geom geometry)
RETURNS bytea[]
LANGUAGE sql AS $$
    SELECT state || $2::bytea
$$;

CREATE OR REPLACE FUNCTION quackgis._union_final(state bytea[])
RETURNS geometry
LANGUAGE plpgsql AS $$
DECLARE
    result geometry;
BEGIN
    IF state IS NULL OR array_length(state, 1) IS NULL THEN
        RETURN NULL;
    END IF;
    -- Build a VALUES list and route through the bridge table to DuckDB.
    EXECUTE format(
        'SELECT st_union_agg(col) FROM (SELECT unnest($1::bytea[]) AS col) t, quackgis._bridge LIMIT 1'
    ) INTO result USING state;
    RETURN result;
END;
$$;

DROP AGGREGATE IF EXISTS st_union_agg(geometry);
CREATE AGGREGATE st_union_agg(geometry) (
    SFUNC = quackgis._union_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._union_final,
    INITCOND = '{}'
);

-- PostGIS ST_Union(geom) aggregate alias
DROP AGGREGATE IF EXISTS st_union(geometry);
CREATE AGGREGATE st_union(geometry) (
    SFUNC = quackgis._union_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._union_final,
    INITCOND = '{}'
);

-- ── st_collect: aggregate geometry collection ───────────────────────────────

CREATE OR REPLACE FUNCTION quackgis._collect_state(state bytea[], geom geometry)
RETURNS bytea[]
LANGUAGE sql AS $$
    SELECT state || $2::bytea
$$;

CREATE OR REPLACE FUNCTION quackgis._collect_final(state bytea[])
RETURNS geometry
LANGUAGE plpgsql AS $$
DECLARE
    result geometry;
BEGIN
    IF state IS NULL OR array_length(state, 1) IS NULL THEN
        RETURN NULL;
    END IF;
    EXECUTE format(
        'SELECT st_collect(col) FROM (SELECT unnest($1::bytea[]) AS col) t, quackgis._bridge LIMIT 1'
    ) INTO result USING state;
    RETURN result;
END;
$$;

DROP AGGREGATE IF EXISTS st_collect(geometry);
CREATE AGGREGATE st_collect(geometry) (
    SFUNC = quackgis._collect_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._collect_final,
    INITCOND = '{}'
);

-- ── st_makeline_agg: points → LineString ────────────────────────────────────

CREATE OR REPLACE FUNCTION quackgis._makeline_state(state bytea[], geom geometry)
RETURNS bytea[]
LANGUAGE sql AS $$
    SELECT state || $2::bytea
$$;

CREATE OR REPLACE FUNCTION quackgis._makeline_final(state bytea[])
RETURNS geometry
LANGUAGE plpgsql AS $$
DECLARE
    result geometry;
BEGIN
    IF state IS NULL OR array_length(state, 1) IS NULL THEN
        RETURN NULL;
    END IF;
    EXECUTE format(
        'SELECT st_makeline_agg(col) FROM (SELECT unnest($1::bytea[]) AS col) t, quackgis._bridge LIMIT 1'
    ) INTO result USING state;
    RETURN result;
END;
$$;

DROP AGGREGATE IF EXISTS st_makeline_agg(geometry);
CREATE AGGREGATE st_makeline_agg(geometry) (
    SFUNC = quackgis._makeline_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._makeline_final,
    INITCOND = '{}'
);

-- PostGIS ST_MakeLine(geom) aggregate alias
DROP AGGREGATE IF EXISTS st_makeline(geometry);
CREATE AGGREGATE st_makeline(geometry) (
    SFUNC = quackgis._makeline_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._makeline_final,
    INITCOND = '{}'
);

-- ── st_envelope_agg: bbox union ─────────────────────────────────────────────

CREATE OR REPLACE FUNCTION quackgis._envelope_state(state bytea[], geom geometry)
RETURNS bytea[]
LANGUAGE sql AS $$
    SELECT state || $2::bytea
$$;

CREATE OR REPLACE FUNCTION quackgis._envelope_final(state bytea[])
RETURNS geometry
LANGUAGE plpgsql AS $$
DECLARE
    result geometry;
BEGIN
    IF state IS NULL OR array_length(state, 1) IS NULL THEN
        RETURN NULL;
    END IF;
    EXECUTE format(
        'SELECT st_envelope_agg(col) FROM (SELECT unnest($1::bytea[]) AS col) t, quackgis._bridge LIMIT 1'
    ) INTO result USING state;
    RETURN result;
END;
$$;

DROP AGGREGATE IF EXISTS st_envelope_agg(geometry);
CREATE AGGREGATE st_envelope_agg(geometry) (
    SFUNC = quackgis._envelope_state,
    STYPE = bytea[],
    FINALFUNC = quackgis._envelope_final,
    INITCOND = '{}'
);
