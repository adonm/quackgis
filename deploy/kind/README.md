# Kind client probes

Current v0.2 deployment smoke tests run QuackGIS as the Rust pgwire binary in
Kind and run client containers as Kubernetes Jobs. This avoids host networking,
ephemeral ports, and local tool drift: clients connect to stable DNS
`quackgis.quackgis.svc.cluster.local:5434` with user `postgres`, database
`quackgis`, and no password in dev mode.

```sh
eval "$(mise activate bash)"
just kind-up
just kind-build-image          # release profile
# or: just kind-build-image-fast  # probe profile, no thin-LTO
just kind-deploy
just kind-probes               # QGIS read/edit + OGR + GeoServer in one wait
```

`just kind-build-image` builds `quackgis-server` locally first and copies only
the release binary into a runtime image, so normal Cargo caches are reused. Use
`just kind-build-image-fast` / `just kind-refresh-fast` for probe triage; they
use Cargo's `probe` profile (release-like, but no thin-LTO/single-codegen-unit)
to cut rebuild latency. Use `just kind-build-image-container` when you need a
clean container-native build.

Run these recipes from an activated mise shell so Kind, kubectl, podman, Cargo,
and Just all come from the repo-pinned environment.

`just kind-probes` starts the maintained QGIS read, QGIS edit, OGR, and
GeoServer Jobs together and waits once, which is the fastest full client-probe
loop after a refresh. Individual probe targets remain available for focused
triage.

`kind-qgis-probe` is the QGIS add-layer/read-feature gate.
`kind-qgis-edit-probe` opens a keyless spatial layer through the QGIS postgres
provider and exercises insert/update/delete/commit with `_quackgis_rowid`.
`kind-ogr-probe` uses GDAL's PostgreSQL driver (`ogrinfo`/`ogr2ogr`) to read a
WKB-backed table, append a GeoJSON layer with `PG_USE_COPY=NO` + `-addfields`,
and assert both SQL and GeoJSON read-back.
`kind-geoserver-probe` uses official GeoServer 3.0.0 plus pgjdbc to register a
PostGIS datastore, publish a WKB-backed layer, verify WFS GeoJSON feature count,
and verify WMS returns a PNG.

`kind-osm-postgis-parity` is opt-in and not part of the default probe loop. It
starts `postgis-osm`, downloads a real Geofabrik OSM extract, loads a named Point
sample into PostGIS, copies it into QuackGIS with `ogr2ogr`/`PG_USE_COPY=NO`,
and compares GeoJSON exports from both databases. Use
`kind-postgis-osm-down` to remove the reference PostGIS deployment.

The `ducklake` PVC on the StatefulSet is intentionally shared by restarts of the
single QuackGIS pod. Future multi-server tests should switch the catalog/data
backend to a shared RWX/object-store setup before scaling replicas.
