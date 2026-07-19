CREATE FOREIGN TABLE remote.worker_identity_export (
  worker_id text,
  catalog_path text,
  snapshot_id bigint,
  row_count bigint,
  data_file_count bigint,
  data_files text
)
SERVER quack_worker
OPTIONS (table 'remote.main.worker_identity_export');

GRANT SELECT ON remote.worker_identity_export TO quackgis_reader;
