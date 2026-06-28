-- SPDX-License-Identifier: Apache-2.0
-- SpatialBench (DuckDB dialect) queries adapted for the sedonadb extension,
-- run over a local DuckLake catalog (DuckDB catalog file + local parquet data).
--
-- Adaptations vs the canonical queries at
-- https://github.com/apache/sedona-spatialbench :
--   * sedonadb's ST_* functions consume ISO-WKB BLOB directly, so we drop the
--     `ST_GeomFromWKB(<column>)` wrapper (the lake columns are already WKB
--     BLOB). Literals use ST_GeomFromText, cast to BLOB to pick our overload
--     unambiguously (DuckDB 1.5 also ships a GEOMETRY-returning ST_GeomFromWKB).
--   * Q5 (ST_Collect aggregate over ARRAY_AGG) is skipped pending a LIST<BLOB>
--     aggregate. Zone queries (Q2/Q4/Q6/Q10/Q11) run only if the zone table
--     exists in the lake.
--
-- Run:  duckdb -unsigned < benchmarks/spatialbench_lake.sql
.bail off
.mode list
.timer on
.output stdout

LOAD ducklake;
LOAD '/var/home/adonm/dev/duckdb_sedona/build/dev/sedonadb.duckdb_extension';
ATTACH IF NOT EXISTS 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data');

-- Sanity: row counts
SELECT 'rows' AS metric, 'trip' AS t, count(*) AS n FROM lake.trip;
SELECT 'rows' AS metric, 'building' AS t, count(*) AS n FROM lake.building;

-- Q1: trips within 50km of Sedona, ordered by distance
SELECT 'Q1' AS q, count(*) AS n FROM (
  SELECT t.t_tripkey FROM lake.trip t
  WHERE st_dwithin(t.t_pickuploc, CAST(st_geomfromtext('POINT (-111.7610 34.8697)') AS BLOB), 0.45)
);

-- Q3: monthly trip stats within ~15km of Sedona (just count groups here)
SELECT 'Q3' AS q, count(*) AS n_groups FROM (
  SELECT DATE_TRUNC('month', t.t_pickuptime) AS pm
  FROM lake.trip t
  WHERE st_dwithin(t.t_pickuploc,
                   CAST(st_geomfromtext('POLYGON((-111.9060 34.7347, -111.6160 34.7347, -111.6160 35.0047, -111.9060 35.0047, -111.9060 34.7347))') AS BLOB),
                   0.045)
  GROUP BY pm
);

-- Q7: route detour — geometric line distance for every trip
SELECT 'Q7' AS q, count(*) AS n, round(avg(st_length(st_makeline(t.t_pickuploc, t.t_dropoffloc))),4) AS avg_line_deg
FROM lake.trip t;

-- Q8: nearby pickups per building within 500m
SELECT 'Q8' AS q, b.b_buildingkey, count(*) AS nearby_pickups
FROM lake.trip t JOIN lake.building b
  ON st_dwithin(t.t_pickuploc, b.b_boundary, 0.0045)
GROUP BY b.b_buildingkey
ORDER BY nearby_pickups DESC;

-- Q9: building self-join IoU pairs
SELECT 'Q9' AS q, count(*) AS overlap_pairs FROM (
  SELECT 1 FROM lake.building b1 JOIN lake.building b2
    ON b1.b_buildingkey < b2.b_buildingkey AND st_intersects(b1.b_boundary, b2.b_boundary)
);

-- Q12-ish: nearest 5 buildings per trip is O(trip*building); with few buildings
-- we report avg distance from each trip to its closest building.
SELECT 'Q12' AS q, round(avg(d),4) AS avg_min_dist_deg FROM (
  SELECT t.t_tripkey, min(st_distance(t.t_pickuploc, b.b_boundary)) AS d
  FROM lake.trip t, lake.building b
  GROUP BY t.t_tripkey
);
