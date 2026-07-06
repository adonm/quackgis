# Kind client probes

Current v0.2 deployment smoke tests run QuackGIS as the Rust pgwire binary in
Kind and run client containers as Kubernetes Jobs. This avoids host networking,
ephemeral ports, and local tool drift: clients connect to stable DNS
`quackgis.quackgis.svc.cluster.local:5434` with user `postgres`, database
`quackgis`, and no password in dev mode.

```sh
just kind-up
just kind-build-image
just kind-deploy
just kind-qgis-probe
just kind-ogr-probe
```

`just kind-build-image` builds `quackgis-server` locally first and copies only
the release binary into a runtime image, so normal Cargo caches are reused. Use
`just kind-build-image-container` when you need a clean container-native build.

`kind-qgis-probe` is the QGIS add-layer/read-feature gate. `kind-ogr-probe`
uses GDAL's PostgreSQL driver (`ogrinfo`/`ogr2ogr`) to read a WKB-backed table,
append a GeoJSON layer with `PG_USE_COPY=NO` + `-addfields`, and assert both SQL
and GeoJSON read-back.

The `ducklake` PVC on the StatefulSet is intentionally shared by restarts of the
single QuackGIS pod. Future multi-server tests should switch the catalog/data
backend to a shared RWX/object-store setup before scaling replicas.
