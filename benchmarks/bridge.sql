-- SPDX-License-Identifier: Apache-2.0
-- Bridge overhead benchmark: local ST_* reimplementation vs the literal Apache
-- SedonaDB kernel (sedona_*) over a sized geometry table. Both paths share the
-- same DuckDB vectorized chunking; the delta is the DuckDB⇄Arrow bridge cost
-- (array build + invoke_with_args + write-back). Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < benchmarks/bridge.sql
.bail off
.timer on

-- (extension loaded via -cmd by the caller)

-- 1M-point table.
CREATE TABLE pts AS SELECT st_point(i, i*2.0) AS g FROM range(0, 1000000) t(i);

.echo off
SELECT 'built 1,000,000 points' AS status;
.echo on

-- Accessors (cheap kernel; overhead-dominated on the sedona side).
SELECT 'dimension'   AS fn, count(*), sum(st_dimension(g))        FROM pts;
SELECT 'sedona_dim'  AS fn, count(*), sum(sedona_st_dimension(g)) FROM pts;
SELECT 'xmin'        AS fn, count(*), sum(st_xmin(g))             FROM pts;
SELECT 'sedona_xmin' AS fn, count(*), sum(sedona_st_xmin(g))      FROM pts;

-- Text serialization (heavier kernel; bridge cost amortized).
SELECT 'astext'      AS fn, count(*), sum(length(st_astext(g)))        FROM pts;
SELECT 'sedona_astext' AS fn, count(*), sum(length(sedona_st_astext(g))) FROM pts;

-- Geometry-producing transform.
SELECT 'segmentize'    AS fn, count(*), sum(st_numpoints(st_segmentize(g, 0.5)))        FROM pts;
SELECT 'sedona_seg'    AS fn, count(*), sum(st_numpoints(sedona_st_segmentize(g, 0.5))) FROM pts;
