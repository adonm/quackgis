# GeoServer example

The maintained GeoServer path is exercised in Kind because it needs a GeoServer
container plus pgjdbc:

```sh
just demo-kind
just kind-geoserver-probe
```

Manual datastore settings for a local or in-cluster GeoServer:

```text
Type: PostGIS
host: 127.0.0.1              # or quackgis.quackgis.svc.cluster.local inside Kind
port: 5434
database: quackgis
schema: public
user: postgres
passwd: <empty>
Expose primary keys: true
```

Publish these feature types:

- `demo_points`
- `demo_polygons`

Smoke URLs after publishing `demo_points` in workspace `quackgis`:

```text
/geoserver/quackgis/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=quackgis:demo_points&outputFormat=application/json
/geoserver/quackgis/wms?service=WMS&version=1.1.0&request=GetMap&layers=quackgis:demo_points&bbox=-1,-1,2,2&width=256&height=256&srs=EPSG:4326&format=image/png
```
