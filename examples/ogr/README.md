# GDAL/OGR example

Start and seed a local demo server:

```sh
just demo-local
```

Inspect the demo layers:

```sh
ogrinfo 'PG:host=127.0.0.1 port=5434 user=postgres dbname=quackgis' demo_points -so
ogrinfo 'PG:host=127.0.0.1 port=5434 user=postgres dbname=quackgis' demo_polygons -so
```

Export demo points to GeoJSON:

```sh
ogr2ogr -f GeoJSON .tmp/demo_points.geojson \
  'PG:host=127.0.0.1 port=5434 user=postgres dbname=quackgis' \
  demo_points
```

The maintained Kind OGR probe also covers a load/read round trip with GDAL's
PostgreSQL driver:

```sh
just kind-ogr-probe
```
