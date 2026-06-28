-- SPDX-License-Identifier: Apache-2.0
-- Backend performance tracking: GEOS topology, spheroid geodesics, raster
-- pixel streaming, and literal-vs-local bridge overhead. These are not
-- competitive benchmarks — they track that each backend is in the expected
-- order of magnitude and that adding a backend does not silently regress
-- the core vector path.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < benchmarks/backends.sql
.bail off
.timer on

-- === GEOS topology (ST_Node, ST_Polygonize, ST_VoronoiPolygons) ===========
-- Build a table of crossing lines and measure GEOS overhead.
CREATE TABLE lines AS
SELECT st_geomfromtext('MULTILINESTRING(('||(i*10)||' 0,'||(i*10+5)||' 5),('||(i*10)||' 5,'||(i*10+5)||' 0))') AS g
FROM range(0, 10000) t(i);

.echo off
SELECT 'built 10,000 crossing-line multilinestrings' AS status;
.echo on

SELECT 'node_10k'        AS op, count(*) FROM (SELECT st_node(g) FROM lines);
SELECT 'voronoi_10k'     AS op, count(*) FROM (SELECT st_voronoipolygons(g) FROM lines);

-- === GEOS MakeValid (bowtie repair) =======================================
CREATE TABLE bowties AS
SELECT st_geomfromtext('POLYGON(('||(i*10)||' 0,'||(i*10+4)||' 4,'||(i*10+4)||' 0,'||(i*10)||' 4,'||(i*10)||' 0))') AS g
FROM range(0, 10000) t(i);

SELECT 'makevalid_10k'   AS op, count(*) FROM (SELECT st_makevalid(g) FROM bowties);

-- === Spheroid geodesics (GeographicLib/Karney) =============================
-- 100k point pairs — the antipodal-safe path.
CREATE TABLE pairs AS
SELECT st_point(i*0.001, 45.0 + i*0.0001) AS a, st_point(i*0.002, -45.0 - i*0.0001) AS b
FROM range(0, 100000) t(i);

SELECT 'spheroid_100k'   AS op, count(*), sum(st_distancespheroid(a, b)) FROM pairs;
SELECT 'sphere_100k'     AS op, count(*), sum(st_distancesphere(a, b)) FROM pairs;

-- === Raster pixel streaming + map algebra =================================
SELECT 'raster_scan'     AS op, count(*), avg(value) FROM st_pixeldata('tests/data/test_raster.asc', 1);

-- === Bridge overhead: literal vs local (routed functions) ==================
-- For routed functions, both st_* and sedona_st_* use the same kernel; the
-- only difference is whether the bridge was invoked. For unrouted functions,
-- st_* uses the local geo-crate path.
CREATE TABLE pts AS SELECT st_point(i, i*2.0) AS g FROM range(0, 100000) t(i);

SELECT 'local_xmin_100k'    AS op, count(*), sum(st_xmin(g)) FROM pts;
SELECT 'literal_xmin_100k'  AS op, count(*), sum(sedona_st_xmin(g)) FROM pts;
SELECT 'local_astext_100k'  AS op, count(*), sum(length(st_astext(g))) FROM pts;
SELECT 'literal_astext_100k' AS op, count(*), sum(length(sedona_st_astext(g))) FROM pts;
