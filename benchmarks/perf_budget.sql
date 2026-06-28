-- SPDX-License-Identifier: Apache-2.0
-- Performance budget tracking: repeatable timing suite for every backend.
-- Reports wall-clock seconds per operation. Not competitive benchmarks —
-- these verify that each backend is in the expected order of magnitude and
-- catch silent regressions.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < benchmarks/perf_budget.sql
.bail off
.timer on
.echo off

-- Helper: print a status line before each section.
.print === Load time ===

.timer off
.print === Build test data ===

-- Build representative datasets.
CREATE TABLE pts_100k AS
    SELECT st_point(i * 0.001, 45.0 + i * 0.0001) AS g
    FROM range(0, 100000) t(i);

CREATE TABLE crossing_lines AS
    SELECT st_geomfromtext(
        'MULTILINESTRING((' || (i*10) || ' 0,' || (i*10+5) || ' 5),'
        || '(' || (i*10) || ' 5,' || (i*10+5) || ' 0))'
    ) AS g
    FROM range(0, 10000) t(i);

CREATE TABLE bowties AS
    SELECT st_geomfromtext(
        'POLYGON((' || (i*10) || ' 0,' || (i*10+4) || ' 4,'
        || (i*10+4) || ' 0,' || (i*10) || ' 4,' || (i*10) || ' 0))'
    ) AS g
    FROM range(0, 10000) t(i);

CREATE TABLE pairs AS
    SELECT st_point(i * 0.001, 45.0 + i * 0.0001) AS a,
           st_point(i * 0.002, -45.0 - i * 0.0001) AS b
    FROM range(0, 100000) t(i);

.print
.print === 1. Bridge overhead: literal vs local (100k points) ===
.timer on

SELECT 'local_xmin'       AS op, count(*), sum(st_xmin(g))       FROM pts_100k;
SELECT 'literal_xmin'     AS op, count(*), sum(sedona_st_xmin(g)) FROM pts_100k;
SELECT 'local_astext'     AS op, count(*), sum(length(st_astext(g))) FROM pts_100k;
SELECT 'literal_astext'   AS op, count(*), sum(length(sedona_st_astext(g))) FROM pts_100k;
SELECT 'local_dimension'  AS op, count(*), sum(st_dimension(g))  FROM pts_100k;
SELECT 'literal_dimension' AS op, count(*), sum(sedona_st_dimension(g)) FROM pts_100k;

.print
.print === 2. GEOS topology (10k geometries) ===

SELECT 'geos_node_10k'    AS op, count(*) FROM (SELECT st_node(g) FROM crossing_lines);
SELECT 'geos_voronoi_10k' AS op, count(*) FROM (SELECT st_voronoipolygons(g) FROM crossing_lines);
SELECT 'geos_makevalid_10k' AS op, count(*) FROM (SELECT st_makevalid(g) FROM bowties);
SELECT 'geos_snap_10k'    AS op, count(*) FROM (SELECT st_snap(g, g, 0.0) FROM bowties);

.print
.print === 3. Spheroid geodesics (100k pairs) ===

SELECT 'spheroid_100k'    AS op, count(*), sum(st_distancespheroid(a, b)) FROM pairs;
SELECT 'sphere_100k'      AS op, count(*), sum(st_distancesphere(a, b))   FROM pairs;
SELECT 'spheroid_dwithin_100k' AS op, count(*), sum(CASE WHEN st_dwithinspheroid(a, b, 5000000.0) THEN 1 ELSE 0 END) FROM pairs;

.print
.print === 4. Raster scan ===

SELECT 'raster_scan'      AS op, count(*), avg(value) FROM st_pixeldata('tests/data/test_raster.asc', 1);
SELECT 'raster_value'     AS op, count(*), sum(st_value('tests/data/test_raster.asc', 1, 0.5, 2.5)) FROM range(0, 1000);

.print
.print === 5. Local geo pipeline (100k points) ===

SELECT 'local_area'       AS op, count(*), sum(st_area(st_buffer(g, 0.001))) FROM pts_100k;
SELECT 'local_centroid'   AS op, count(*), sum(st_x(st_centroid(st_buffer(g, 0.001)))) FROM pts_100k;
SELECT 'local_simplify'   AS op, count(*), sum(st_numpoints(st_simplify(st_buffer(g, 0.01), 0.0001))) FROM pts_100k;

.print
.print === 6. Aggregate (100k points) ===

SELECT 'agg_collect'      AS op, count(*) FROM (SELECT st_collect(g) FROM pts_100k);
SELECT 'agg_envelope'     AS op, count(*) FROM (SELECT st_envelope_agg(g) FROM pts_100k);
SELECT 'agg_union'        AS op, count(*) FROM (SELECT st_union_agg(st_buffer(g, 0.0001)) FROM pts_100k);
SELECT 'agg_makeline'     AS op, count(*) FROM (SELECT st_makeline_agg(g) FROM pts_100k);

.print
.print === 7. Table function (100k points) ===

SELECT 'tbl_dump'         AS op, count(*) FROM (SELECT st_dump(st_collect(g)) FROM pts_100k);
SELECT 'tbl_dumppoints'   AS op, count(*) FROM (SELECT st_dumppoints(g) FROM pts_100k LIMIT 10000);

.timer off
.print
.print === 8. GEOS overlay fallback (10k bowties × valid polygon) ===
.timer on

-- Intersection / union / difference on self-intersecting polygons.
-- Local geo will panic on the bowties → GEOS fallback path is exercised.
SELECT 'overlay_intersection_bowtie' AS op, count(*) FROM (SELECT st_intersection(g, st_geomfromtext('POLYGON((0 0,100 0,100 100,0 100,0 0))')) FROM bowties);
SELECT 'overlay_union_bowtie'        AS op, count(*) FROM (SELECT st_union(g, st_geomfromtext('POLYGON((0 0,100 0,100 100,0 100,0 0))')) FROM bowties);
SELECT 'overlay_difference_bowtie'   AS op, count(*) FROM (SELECT st_difference(st_geomfromtext('POLYGON((0 0,100 0,100 100,0 100,0 0))'), g) FROM bowties);

-- ContainsProperly: interior vs boundary.
SELECT 'containsproperly_interior' AS op, count(*) FROM (SELECT 1 FROM pts_100k WHERE st_containsproperly(st_geomfromtext('POLYGON((0 0,100 0,100 100,0 100,0 0))'), g));

-- DumpRings on 10k polygons.
SELECT 'dumprings_10k' AS op, count(*) FROM (SELECT 1 FROM st_dumprings(st_buffer(st_point(0,0), 1.0)));

.timer off
.print
.print === Done ===
