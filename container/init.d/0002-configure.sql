-- SPDX-License-Identifier: Apache-2.0
--
-- QuackGIS DuckDB + DuckLake configuration.
-- Runs after pg_ducklake's own init script (0001-install-pg_ducklake.sql).

-- Set DuckLake data path (where new tables store Parquet data).
DO $$
DECLARE
    data_path text := '/var/lib/quackgis/data/';
BEGIN
    PERFORM set_config('ducklake.default_table_path', data_path, false);
    EXECUTE format('ALTER DATABASE %I SET ducklake.default_table_path = %L',
                   current_database(), data_path);
END $$;

-- Set DuckDB thread count.
DO $$
BEGIN
    PERFORM set_config('ducklake.threads', '-1', false);
    EXECUTE format('ALTER DATABASE %I SET ducklake.threads = %L',
                   current_database(), '-1');
END $$;

-- Create QuackGIS schema.
CREATE SCHEMA IF NOT EXISTS quackgis;

-- PostGIS compatibility catalog (created here, not in extension SQL, to avoid
-- pg_ducklake planner hook issues during CREATE EXTENSION).
CREATE TABLE IF NOT EXISTS spatial_ref_sys (
    srid      integer PRIMARY KEY,
    auth_name varchar(256),
    auth_srid integer,
    srtext    varchar(2048),
    proj4text varchar(2048)
);

CREATE OR REPLACE FUNCTION postgis_version()
RETURNS text LANGUAGE SQL IMMUTABLE AS $$ SELECT '3.4 QUACKGIS' $$;

CREATE OR REPLACE FUNCTION postgis_full_version()
RETURNS text LANGUAGE SQL IMMUTABLE AS $$ SELECT 'QUACKGIS (sedonadb engine)' $$;
