-- SPDX-License-Identifier: Apache-2.0
-- Full SpatialBench run over the local DuckLake, with bbox-prefiltered joins
-- so the spatial joins (Q4/Q6/Q8/Q9/Q10/Q11) are feasible. See
-- benchmarks/BENCHMARKS.md for the methodology and the naive-cross-join
-- finding that motivates the prefilter.
.bail off
.mode list
.timer on

LOAD ducklake;
LOAD '/var/home/adonm/dev/duckdb_sedona/build/dev/sedonadb.duckdb_extension';
ATTACH IF NOT EXISTS 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data');

-- Materialize bbox columns once (the prefilter index).
CREATE OR REPLACE TABLE lake.trip_bbox AS
  SELECT t_tripkey, t_pickuploc, t_dropoffloc,
         st_xmin(t_pickuploc) pxmin, st_xmax(t_pickuploc) pxmax,
         st_ymin(t_pickuploc) pymin, st_ymax(t_pickuploc) pymax
  FROM lake.trip;
CREATE OR REPLACE TABLE lake.zone_bbox AS
  SELECT z_zonekey, z_name, z_boundary,
         st_xmin(z_boundary) zxmin, st_xmax(z_boundary) zxmax,
         st_ymin(z_boundary) zymin, st_ymax(z_boundary) zymax
  FROM lake.zone;
CREATE OR REPLACE TABLE lake.building_bbox AS
  SELECT b_buildingkey, b_name, b_boundary,
         st_xmin(b_boundary) bxmin, st_xmax(b_boundary) bxmax,
         st_ymin(b_boundary) bymin, st_ymax(b_boundary) bymax
  FROM lake.building;

-- Q1: trips within 50km of Sedona
SELECT 'Q1' AS q, count(*) AS n FROM (
  SELECT t_tripkey FROM lake.trip_bbox t
  WHERE st_dwithin(t.t_pickuploc, CAST(st_geomfromtext('POINT (-111.7610 34.8697)') AS BLOB), 0.45)
);

-- Q2: trips starting in Coconino County (single zone)
SELECT 'Q2' AS q, count(*) AS n FROM lake.trip t
WHERE st_intersects(t.t_pickuploc,
  (SELECT z.z_boundary FROM lake.zone z WHERE z.z_name = 'Coconino County' LIMIT 1));

-- Q4: pickup zone for top-1000 trips by tip (bbox-prefiltered)
SELECT 'Q4' AS q, count(*) AS zones_hit FROM (
  SELECT z.z_zonekey
  FROM (SELECT t_pickuploc FROM lake.trip ORDER BY t_tip DESC, t_tripkey ASC LIMIT 1000) tt
  JOIN lake.zone_bbox z
    ON st_xmin(tt.t_pickuploc) <= z.zxmax AND st_xmax(tt.t_pickuploc) >= z.zxmin
   AND st_ymin(tt.t_pickuploc) <= z.zymax AND st_ymax(tt.t_pickuploc) >= z.zymin
  WHERE st_within(tt.t_pickuploc, z.z_boundary)
  GROUP BY z.z_zonekey
);

-- Q7: geometric trip length (full scan)
SELECT 'Q7' AS q, count(*) AS n, round(avg(st_length(st_makeline(t_pickuploc, t_dropoffloc))),5) AS avg_len
FROM lake.trip;

-- Q8: nearby pickups per building within 500m (bbox-prefiltered, expanded by d)
SELECT 'Q8' AS q, count(*) AS buildings_with_pickups, sum(c) AS total_nearby FROM (
  SELECT b.b_buildingkey, count(*) AS c
  FROM lake.trip_bbox t JOIN lake.building_bbox b
    ON t.pxmin <= b.bxmax + 0.0045 AND t.pxmax >= b.bxmin - 0.0045
   AND t.pymin <= b.bymax + 0.0045 AND t.pymax >= b.bymin - 0.0045
  WHERE st_dwithin(t.t_pickuploc, b.b_boundary, 0.0045)
  GROUP BY b.b_buildingkey
);

-- Q9: building self-join IoU pairs (bbox-prefiltered)
SELECT 'Q9' AS q, count(*) AS overlap_pairs FROM (
  SELECT b1.b_buildingkey, b2.b_buildingkey
  FROM lake.building_bbox b1 JOIN lake.building_bbox b2
    ON b1.b_buildingkey < b2.b_buildingkey
   AND b1.bxmin <= b2.bxmax AND b1.bxmax >= b2.bxmin
   AND b1.bymin <= b2.bymax AND b1.bymax >= b2.bymin
  WHERE st_intersects(b1.b_boundary, b2.b_boundary)
);

-- Q10: trips per pickup zone (bbox-prefiltered)
SELECT 'Q10' AS q, count(*) AS zones_with_trips, sum(n) AS total_matches FROM (
  SELECT z.z_zonekey, count(*) AS n
  FROM lake.trip_bbox t JOIN lake.zone_bbox z
    ON t.pxmin <= z.zxmax AND t.pxmax >= z.zxmin
   AND t.pymin <= z.zymax AND t.pymax >= z.zymin
  WHERE st_within(t.t_pickuploc, z.z_boundary)
  GROUP BY z.z_zonekey
);
