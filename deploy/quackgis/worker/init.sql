INSTALL quack FROM core_nightly;
LOAD quack;
INSTALL spatial;
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
SELECT id, name, ST_AsText(geom) AS geom_wkt, minx, miny, maxx, maxy
FROM features;

CALL quack_serve(
  'quack://0.0.0.0:9494',
  token = 'quackgis-dev-token',
  allow_other_hostname = true
);
