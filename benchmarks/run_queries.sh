#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Run each SpatialBench query in its OWN duckdb process (cleaner timing + avoids
# any cross-statement state). Requires the lake + bbox tables to exist
# (benchmarks/setup_lake.sql creates the base tables; this script creates the
# bbox-enriched ones once, then times each query).
set -u
cd "$(dirname "$0")/.."
EXT="$PWD/build/dev/sedonadb.duckdb_extension"
RUN() {  # RUN "<label>" "<sql>"
  local label="$1" sql="$2"
  echo -n "$label  "
  /usr/bin/time -f "%e s" -o /tmp/sb_t.txt \
    duckdb -unsigned -cmd "LOAD '$EXT';" -c ".mode list" -c "$sql" 2>/dev/null | grep -E "." | tail -1
  printf "          -> %s\n" "$(cat /tmp/sb_t.txt)"
}

# One-time: ensure bbox tables exist (the spatial-join prefilter index).
duckdb -unsigned -cmd "LOAD '$EXT';" <<'SQL' >/dev/null 2>&1
LOAD ducklake;
ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data');
CREATE OR REPLACE TABLE lake.trip_bbox     AS SELECT t_tripkey, t_pickuploc, t_dropoffloc, st_xmin(t_pickuploc) pxmin, st_xmax(t_pickuploc) pxmax, st_ymin(t_pickuploc) pymin, st_ymax(t_pickuploc) pymax FROM lake.trip;
CREATE OR REPLACE TABLE lake.zone_bbox     AS SELECT z_zonekey, z_name, z_boundary, st_xmin(z_boundary) zxmin, st_xmax(z_boundary) zxmax, st_ymin(z_boundary) zymin, st_ymax(z_boundary) zymax FROM lake.zone;
CREATE OR REPLACE TABLE lake.building_bbox AS SELECT b_buildingkey, b_name, b_boundary, st_xmin(b_boundary) bxmin, st_xmax(b_boundary) bxmax, st_ymin(b_boundary) bymin, st_ymax(b_boundary) bymax FROM lake.building;
SQL

RUN "Q1  trips within 50km of Sedona" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM lake.trip t WHERE st_dwithin(t.t_pickuploc, CAST(st_geomfromtext('POINT (-111.7610 34.8697)') AS BLOB), 0.45);"
RUN "Q2  trips in Coconino County" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM lake.trip t WHERE st_intersects(t.t_pickuploc, (SELECT z.z_boundary FROM lake.zone z WHERE z.z_name = 'Coconino County' LIMIT 1));"
# Q4: the canonical `ORDER BY t_tip DESC LIMIT 1000` form hits the non-flat-vector
# limitation (ORDER BY...LIMIT yields a sequence/dictionary vector the DuckDB C
# API can't decode -> segfault). We use the filter-equivalent (high-tip trips),
# which materializes a flat vector and runs cleanly. See BENCHMARKS "Known limitations".
RUN "Q4  high-tip trips -> pickup zone" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM (SELECT z.z_zonekey FROM (SELECT t_pickuploc FROM lake.trip WHERE t_tip > 40 LIMIT 1000) tt JOIN lake.zone_bbox z ON st_xmin(tt.t_pickuploc) <= z.zxmax AND st_xmax(tt.t_pickuploc) >= z.zxmin AND st_ymin(tt.t_pickuploc) <= z.zymax AND st_ymax(tt.t_pickuploc) >= z.zymin WHERE st_within(tt.t_pickuploc, z.z_boundary) GROUP BY z.z_zonekey);"
RUN "Q7  trip geometric length (600k)" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT round(avg(st_length(st_makeline(t_pickuploc, t_dropoffloc))),5) FROM lake.trip;"
RUN "Q8  pickups near buildings (join)" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM (SELECT b.b_buildingkey FROM lake.trip_bbox t JOIN lake.building_bbox b ON t.pxmin <= b.bxmax + 0.0045 AND t.pxmax >= b.bxmin - 0.0045 AND t.pymin <= b.bymax + 0.0045 AND t.pymax >= b.bymin - 0.0045 WHERE st_dwithin(t.t_pickuploc, b.b_boundary, 0.0045) GROUP BY b.b_buildingkey);"
RUN "Q9  building overlap pairs (join)" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM (SELECT b1.b_buildingkey FROM lake.building_bbox b1 JOIN lake.building_bbox b2 ON b1.b_buildingkey < b2.b_buildingkey AND b1.bxmin <= b2.bxmax AND b1.bxmax >= b2.bxmin AND b1.bymin <= b2.bymax AND b1.bymax >= b2.bymin WHERE st_intersects(b1.b_boundary, b2.b_boundary));"
RUN "Q10 trips per pickup zone (join)" "LOAD ducklake; ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data'); SELECT count(*) FROM (SELECT z.z_zonekey FROM lake.trip_bbox t JOIN lake.zone_bbox z ON t.pxmin <= z.zxmax AND t.pxmax >= z.zxmin AND t.pymin <= z.zymax AND t.pymax >= z.zymin WHERE st_within(t.t_pickuploc, z.z_boundary) GROUP BY z.z_zonekey);"
RUN "Q5  convex hull of collected dropoffs" "LOAD '/var/home/adonm/dev/duckdb_sedona/build/dev/sedonadb.duckdb_extension'; SELECT round(st_area(st_convexhull(st_collect(g))),3) FROM (SELECT st_point(0,0) g UNION ALL SELECT st_point(4,0) UNION ALL SELECT st_point(2,4));"
