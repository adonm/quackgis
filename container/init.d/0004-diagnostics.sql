-- SPDX-License-Identifier: Apache-2.0
--
-- QuackGIS diagnostics (simplified for pg_ducklake compatibility).

CREATE OR REPLACE FUNCTION quackgis.diagnostics()
RETURNS TABLE(key text, value text)
LANGUAGE plpgsql AS $$
BEGIN
    RETURN QUERY SELECT 'postgres_version'::text, version();
    RETURN QUERY SELECT 'pg_ducklake', extversion
        FROM pg_extension WHERE extname = 'pg_ducklake';
    RETURN QUERY SELECT 'ducklake_data_path', current_setting('ducklake.default_table_path', true);
    RETURN QUERY SELECT 'duckdb_threads', current_setting('ducklake.threads', true);
    RETURN QUERY SELECT 'rewrite_mode', current_setting('quackgis.rewrite_mode', true);
    RETURN QUERY SELECT 'bridge_table', 'quackgis._bridge'::text;
    RETURN;
END $$;

CREATE OR REPLACE FUNCTION quackgis.smoke_check()
RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    result text;
BEGIN
    SELECT st_astext(st_geomfromtext('POINT(0 0)')) INTO result;
    RETURN result;
END $$;
