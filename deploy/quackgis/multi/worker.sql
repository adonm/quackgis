LOAD quack;
LOAD spatial;
LOAD ducklake;

ATTACH 'ducklake:/lake/catalog.ducklake' AS lake (READ_ONLY);

CREATE VIEW features_export AS
SELECT id, name, geom_wkb AS geom, minx, miny, maxx, maxy
FROM lake.public.features;

CREATE VIEW geometry_contract_export AS
SELECT 1::BIGINT AS id, ST_AsWKB(ST_GeomFromText('POINT(-123.10 49.20)')) AS geom
UNION ALL
SELECT 2::BIGINT AS id, NULL::BLOB AS geom;

CREATE VIEW geometry_malformed_export AS
SELECT 1::BIGINT AS id, from_hex('00') AS geom;

CREATE VIEW geometry_wrong_family_export AS
SELECT
  1::BIGINT AS id,
  ST_AsWKB(ST_GeomFromText('POLYGON((-123 49,-122 49,-122 50,-123 49))')) AS geom;

CREATE VIEW worker_identity_export AS
SELECT
  getenv('QUACKGIS_WORKER_ID') AS worker_id,
  '/lake/catalog.ducklake'::VARCHAR AS catalog_path,
  (SELECT MAX(snapshot_id) FROM ducklake_snapshots('lake')) AS snapshot_id,
  (SELECT COUNT(*) FROM lake.public.features) AS row_count,
  (
    SELECT COUNT(*)
    FROM ducklake_list_files('lake', 'features', schema => 'public')
  ) AS data_file_count,
  (
    SELECT string_agg(data_file, ',' ORDER BY data_file)
    FROM ducklake_list_files('lake', 'features', schema => 'public')
  ) AS data_files;

CALL quack_serve(
  'quack://127.0.0.1:9494',
  token = 'quackgis-dev-token',
  allow_other_hostname = false
);
