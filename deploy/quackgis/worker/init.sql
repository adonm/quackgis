LOAD quack;
LOAD spatial;

CREATE TABLE IF NOT EXISTS features (
  id BIGINT PRIMARY KEY,
  name VARCHAR NOT NULL,
  geom GEOMETRY NOT NULL,
  minx DOUBLE NOT NULL,
  miny DOUBLE NOT NULL,
  maxx DOUBLE NOT NULL,
  maxy DOUBLE NOT NULL
);

INSERT OR REPLACE INTO features VALUES
  (1, 'west', ST_GeomFromText('POINT(-123.10 49.20)'), -123.10, 49.20, -123.10, 49.20),
  (2, 'east', ST_GeomFromText('POINT(-122.90 49.25)'), -122.90, 49.25, -122.90, 49.25),
  (3, 'south', ST_GeomFromText('POINT(-123.05 48.90)'), -123.05, 48.90, -123.05, 48.90);

CREATE OR REPLACE VIEW features_export AS
SELECT id, name, ST_AsWKB(geom) AS geom, minx, miny, maxx, maxy
FROM features;

CREATE OR REPLACE VIEW geometry_contract_export AS
SELECT 1::BIGINT AS id, ST_AsWKB(ST_GeomFromText('POINT(-123.10 49.20)')) AS geom
UNION ALL
SELECT 2::BIGINT AS id, NULL::BLOB AS geom;

CREATE OR REPLACE VIEW geometry_malformed_export AS
SELECT 1::BIGINT AS id, from_hex('00') AS geom;

CREATE OR REPLACE VIEW geometry_wrong_family_export AS
SELECT
  1::BIGINT AS id,
  ST_AsWKB(ST_GeomFromText('POLYGON((-123 49,-122 49,-122 50,-123 49))')) AS geom;

CALL quack_serve(
  'quack://127.0.0.1:9494',
  token = 'quackgis-dev-token',
  allow_other_hostname = false
);
