-- SPDX-License-Identifier: Apache-2.0
-- Reproducible setup for a LOCAL DuckLake: a DuckDB file holds the catalog
-- (metadata) and a local folder holds the table data as Parquet. SpatialBench
-- tables are ingested from generated parquet.
--
-- Prereqs:
--   * DuckDB 1.5.x CLI on PATH
--   * sedonadb extension packaged at build/dev/sedonadb.duckdb_extension
--   * SpatialBench parquet under build/spatialbench-sf0.1 (trip + dims) and
--     build/spatialbench-sf1-fragments/building.parquet
--     (generate with the spatialbench-cli; see benchmarks/run.sh)
--
-- Run:  duckdb -unsigned < benchmarks/setup_lake.sql
.bail off
LOAD ducklake;
LOAD '/var/home/adonm/dev/duckdb_sedona/build/dev/sedonadb.duckdb_extension';

-- Recreate the lake from scratch.
DETACH IF EXISTS lake;
ATTACH 'ducklake:build/lake/catalog.duckdb' AS lake (DATA_PATH 'build/lake/data');
DROP TABLE IF EXISTS lake.trip;
DROP TABLE IF EXISTS lake.building;
DROP TABLE IF EXISTS lake.customer;
DROP TABLE IF EXISTS lake.driver;
DROP TABLE IF EXISTS lake.vehicle;

CREATE TABLE lake.trip     AS SELECT * FROM read_parquet('build/spatialbench-sf0.1/trip.parquet');
CREATE TABLE lake.building AS SELECT * FROM read_parquet('build/spatialbench-sf1-fragments/building.parquet');
CREATE TABLE lake.customer AS SELECT * FROM read_parquet('build/spatialbench-sf0.1/customer.parquet');
CREATE TABLE lake.driver   AS SELECT * FROM read_parquet('build/spatialbench-sf0.1/driver.parquet');
CREATE TABLE lake.vehicle  AS SELECT * FROM read_parquet('build/spatialbench-sf0.1/vehicle.parquet');

SELECT 'trip' AS t, count(*) AS n FROM lake.trip
UNION ALL SELECT 'building', count(*) FROM lake.building
UNION ALL SELECT 'customer', count(*) FROM lake.customer;
