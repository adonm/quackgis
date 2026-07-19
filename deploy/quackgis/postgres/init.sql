\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS duckdb_fdw;

CREATE ROLE quackgis_reader LOGIN PASSWORD 'quackgis-reader-dev';
CREATE SCHEMA remote AUTHORIZATION postgres;

CREATE SERVER quack_worker
  FOREIGN DATA WRAPPER duckdb_fdw
  OPTIONS (quack_host 'worker:9494', disable_ssl 'true');

CREATE USER MAPPING FOR postgres
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE USER MAPPING FOR quackgis_reader
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE FOREIGN TABLE remote.features_export (
  id bigint,
  name text,
  geom_wkt text,
  minx double precision,
  miny double precision,
  maxx double precision,
  maxy double precision
)
SERVER quack_worker
OPTIONS (table 'remote.main.features_export');

-- Development-only bridge. P2 replaces WKT with native WKB/EWKB conversion in
-- the FDW and pushes bbox candidates before this local exact expression.
CREATE VIEW public.features_unbounded AS
SELECT
  id,
  name,
  ST_SetSRID(ST_GeomFromText(geom_wkt), 4326)::geometry(Point, 4326) AS geom,
  minx,
  miny,
  maxx,
  maxy
FROM remote.features_export;

GRANT USAGE ON SCHEMA public, remote TO quackgis_reader;
GRANT USAGE ON FOREIGN SERVER quack_worker TO quackgis_reader;
GRANT SELECT ON remote.features_export, public.features_unbounded TO quackgis_reader;
