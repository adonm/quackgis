-- SPDX-License-Identifier: Apache-2.0
--
-- QuackGIS DuckLake spatial layout helpers (M7+).
--
-- With pg_ducklake, DuckLake tables are native PG tables (USING ducklake).
-- No per-session ATTACH needed. These helpers create tables with spatial
-- layout columns and provide query/count utilities.

-- ── create_spatial_table: DuckLake table with layout columns ────────────────
--
-- Usage:
--   SELECT quackgis.create_spatial_table(
--       'public.parcels',           -- target table name
--       'SELECT id, geom FROM raw',  -- source query (must include geom)
--       zoom := 8, bits := 16
--   );

CREATE OR REPLACE FUNCTION quackgis.create_spatial_table(
    target text,
    source_query text,
    zoom int DEFAULT 8,
    bits int DEFAULT 16,
    partition_by text DEFAULT 'spatial_cell'
)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
    v_ctas text;
BEGIN
    -- CTAS with layout columns, using USING ducklake for native storage.
    v_ctas := format(
        'CREATE TABLE %s USING ducklake AS
         SELECT *,
                st_xmin(geom) AS minx,
                st_ymin(geom) AS miny,
                st_xmax(geom) AS maxx,
                st_ymax(geom) AS maxy,
                st_quadkey(geom, %s) AS spatial_cell,
                st_hilbert(geom, %s) AS spatial_sort
         FROM (%s)
         ORDER BY spatial_sort',
        target, zoom, bits, source_query
    );

    EXECUTE v_ctas;

    -- Set partitioning via pg_ducklake's native proc.
    IF partition_by IS NOT NULL AND partition_by != '' THEN
        EXECUTE format('CALL ducklake.set_partition(%L, %L)', target, partition_by);
    END IF;

    RETURN format('created %s (zoom=%s, bits=%s, partition=%s)',
                  target, zoom, bits, partition_by);
EXCEPTION WHEN OTHERS THEN
    RETURN 'error: ' || SQLERRM;
END;
$$;

-- ── spatial_query_count: three-stage query ──────────────────────────────────

CREATE OR REPLACE FUNCTION quackgis.spatial_query_count(
    table_name text,
    q_wkt text,
    q_distance double precision DEFAULT 0.0,
    zoom int DEFAULT 8,
    predicate text DEFAULT 'st_intersects'
)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
    v_count bigint;
BEGIN
    EXECUTE format(
        'SELECT count(*) FROM %s p,
         (SELECT st_geomfromtext(''%s'') AS qgeom,
                 st_xmin(st_geomfromtext(''%s'')) AS qminx,
                 st_ymin(st_geomfromtext(''%s'')) AS qminy,
                 st_xmax(st_geomfromtext(''%s'')) AS qmaxx,
                 st_ymax(st_geomfromtext(''%s'')) AS qmaxy
         ) q
         WHERE p.spatial_cell IN (
             SELECT quadkey FROM st_covering_quadkeys(
                 st_makeenvelope(q.qminx - %s, q.qminy - %s, q.qmaxx + %s, q.qmaxy + %s),
                 %s, 10000
             )
         )
         AND p.maxx >= q.qminx - %s AND p.minx <= q.qmaxx + %s
         AND p.maxy >= q.qminy - %s AND p.miny <= q.qmaxy + %s
         AND %s(p.geom, q.qgeom)',
        table_name, q_wkt, q_wkt, q_wkt, q_wkt,
        q_distance, q_distance, q_distance, q_distance,
        zoom,
        q_distance, q_distance, q_distance, q_distance,
        predicate
    ) INTO v_count;

    RETURN v_count;
END;
$$;

-- ── exact_query_count: exact-only baseline ──────────────────────────────────

CREATE OR REPLACE FUNCTION quackgis.exact_query_count(
    table_name text,
    q_wkt text,
    predicate text DEFAULT 'st_intersects'
)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
    v_count bigint;
BEGIN
    EXECUTE format(
        'SELECT count(*) FROM %s p,
         (SELECT st_geomfromtext(''%s'') AS qgeom) q
         WHERE %s(p.geom, q.qgeom)',
        table_name, q_wkt, predicate
    ) INTO v_count;
    RETURN v_count;
END;
$$;

-- ── pruning_report: compare three-stage vs exact ────────────────────────────

CREATE OR REPLACE FUNCTION quackgis.pruning_report(
    table_name text,
    q_wkt text,
    zoom int DEFAULT 8,
    predicate text DEFAULT 'st_intersects'
)
RETURNS TABLE(strategy text, row_count bigint, note text)
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
    v_exact bigint;
    v_staged bigint;
BEGIN
    v_exact := quackgis.exact_query_count(table_name, q_wkt, predicate);
    v_staged := quackgis.spatial_query_count(table_name, q_wkt, 0.0, zoom, predicate);

    RETURN QUERY SELECT 'exact_only'::text, v_exact, 'full scan + exact predicate'::text;
    RETURN QUERY SELECT 'three_stage'::text, v_staged,
        format('cell(bbox)+exact at zoom %s', zoom)::text;

    IF v_exact = v_staged THEN
        RETURN QUERY SELECT 'parity'::text, 0::bigint,
            'PASS: three-stage returns same count as exact'::text;
    ELSE
        RETURN QUERY SELECT 'parity'::text, abs(v_exact - v_staged)::bigint,
            format('FAIL: exact=%s staged=%s', v_exact, v_staged)::text;
    END IF;
END;
$$;
