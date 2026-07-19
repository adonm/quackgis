LOAD ducklake;
LOAD spatial;

SET ducklake_default_data_inlining_row_limit = 0;

ATTACH 'ducklake:/lake/catalog.ducklake' AS lake (
  DATA_PATH '/lake/data',
  DATA_INLINING_ROW_LIMIT 0
);

CREATE SCHEMA IF NOT EXISTS lake.public;

CREATE TABLE IF NOT EXISTS lake.public.features (
  id BIGINT,
  name VARCHAR,
  geom_wkb BLOB,
  minx DOUBLE,
  miny DOUBLE,
  maxx DOUBLE,
  maxy DOUBLE
);

INSERT INTO lake.public.features
SELECT seed.*
FROM (
  VALUES
    (1, 'west', ST_AsWKB(ST_GeomFromText('POINT(-123.10 49.20)')), -123.10, 49.20, -123.10, 49.20),
    (2, 'east', ST_AsWKB(ST_GeomFromText('POINT(-122.90 49.25)')), -122.90, 49.25, -122.90, 49.25),
    (3, 'south', ST_AsWKB(ST_GeomFromText('POINT(-123.05 48.90)')), -123.05, 48.90, -123.05, 48.90)
) AS seed(id, name, geom_wkb, minx, miny, maxx, maxy)
WHERE NOT EXISTS (SELECT 1 FROM lake.public.features);

SELECT
  COUNT(*) AS rows,
  SUM(id) AS id_sum,
  (SELECT MAX(snapshot_id) FROM ducklake_snapshots('lake')) AS snapshot_id
FROM lake.public.features;
