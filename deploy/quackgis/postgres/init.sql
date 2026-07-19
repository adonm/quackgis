\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS duckdb_fdw;

CREATE ROLE quackgis_reader LOGIN PASSWORD 'quackgis-reader-dev';
CREATE ROLE quackgis_features LOGIN PASSWORD 'quackgis-features-dev';
CREATE ROLE quackgis_tiles LOGIN PASSWORD 'quackgis-tiles-dev';

ALTER ROLE quackgis_reader SET default_transaction_read_only = on;
ALTER ROLE quackgis_features SET default_transaction_read_only = on;
ALTER ROLE quackgis_tiles SET default_transaction_read_only = on;
ALTER ROLE quackgis_features SET statement_timeout = '30s';
ALTER ROLE quackgis_tiles SET statement_timeout = '30s';
ALTER ROLE quackgis_features IN DATABASE quackgis
  SET search_path = feature_metadata, public;
ALTER ROLE quackgis_reader IN DATABASE quackgis
  SET search_path = feature_metadata, public;

CREATE SCHEMA remote AUTHORIZATION postgres;

CREATE SERVER quack_worker
  FOREIGN DATA WRAPPER duckdb_fdw
  OPTIONS (quack_host '127.0.0.1:9494', disable_ssl 'true');

CREATE USER MAPPING FOR postgres
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE USER MAPPING FOR quackgis_reader
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE USER MAPPING FOR quackgis_features
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE USER MAPPING FOR quackgis_tiles
  SERVER quack_worker
  OPTIONS (quack_token 'quackgis-dev-token');

CREATE FOREIGN TABLE remote.features_export (
  id bigint,
  name text,
  geom geometry(Point, 4326),
  minx double precision,
  miny double precision,
  maxx double precision,
  maxy double precision
)
SERVER quack_worker
OPTIONS (table 'remote.main.features_export');

CREATE FOREIGN TABLE remote.geometry_contract_export (
  id bigint,
  geom geometry(Point, 4326)
)
SERVER quack_worker
OPTIONS (table 'remote.main.geometry_contract_export');

CREATE FOREIGN TABLE remote.geometry_malformed_export (
  id bigint,
  geom geometry(Point, 4326)
)
SERVER quack_worker
OPTIONS (table 'remote.main.geometry_malformed_export');

CREATE FOREIGN TABLE remote.geometry_wrong_family_export (
  id bigint,
  geom geometry(Point, 4326)
)
SERVER quack_worker
OPTIONS (table 'remote.main.geometry_wrong_family_export');

CREATE VIEW public.features AS
SELECT
  id,
  name,
  geom
FROM remote.features_export;

CREATE SCHEMA feature_metadata AUTHORIZATION postgres;

CREATE TABLE feature_metadata.layer_extents (
  schema_name text NOT NULL,
  relation_name text NOT NULL,
  geometry_column text NOT NULL,
  extent box2d NOT NULL,
  PRIMARY KEY (schema_name, relation_name, geometry_column)
);

INSERT INTO feature_metadata.layer_extents VALUES
  ('public', 'features', 'geom', 'BOX(-123.1 48.9,-122.9 49.25)'::box2d);

CREATE FUNCTION feature_metadata.st_estimatedextent(text, text, text)
RETURNS box2d
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
  SELECT coalesce(
    (
      SELECT extent
      FROM feature_metadata.layer_extents
      WHERE schema_name = $1
        AND relation_name = $2
        AND geometry_column = $3
    ),
    'BOX(-180 -90,180 90)'::box2d
  )
$$;

REVOKE ALL ON FUNCTION feature_metadata.st_estimatedextent(text, text, text)
  FROM PUBLIC;
GRANT USAGE ON SCHEMA feature_metadata TO quackgis_features, quackgis_reader;
GRANT EXECUTE ON FUNCTION feature_metadata.st_estimatedextent(text, text, text)
  TO quackgis_features, quackgis_reader;

GRANT USAGE ON SCHEMA public, remote TO quackgis_reader;
GRANT USAGE ON FOREIGN SERVER quack_worker TO quackgis_reader;
GRANT SELECT ON
  remote.features_export,
  remote.geometry_contract_export,
  remote.geometry_malformed_export,
  remote.geometry_wrong_family_export,
  public.features
TO quackgis_reader;

GRANT CONNECT ON DATABASE quackgis TO quackgis_features, quackgis_tiles;
GRANT USAGE ON SCHEMA public TO quackgis_features, quackgis_tiles;
GRANT SELECT ON public.features TO quackgis_features, quackgis_tiles;
