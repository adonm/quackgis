# Kind client probes

Current v0.2 deployment smoke tests run QuackGIS as the Rust pgwire binary in
Kind and run client containers as Kubernetes Jobs. This avoids host networking,
ephemeral ports, and local tool drift: clients connect to stable DNS
`quackgis.quackgis.svc.cluster.local:5434` with user `postgres`, database
`quackgis`, and no password in dev mode.

```sh
eval "$(mise activate bash)"
just kind-ready                # validate Podman/Kind and create/reuse cluster
just demo-kind                 # deploy, seed stable public.demo_* layers, print hints
just kind-compatibility        # build/deploy + QGIS read/edit, OGR, GeoServer
```

`mise.toml` pins Rust, Just, Kind, kubectl, Helm, and cargo-nextest; Podman is the
host runtime and is selected by `CONTAINER_ENGINE=podman` /
`KIND_EXPERIMENTAL_PROVIDER=podman`. `just kind-up` is idempotent and reuses the
`quackgis` cluster if it already exists. `just kind-status` prints the current
nodes and QuackGIS namespace resources.

The maintained probe programs live under `deploy/kind/probes/` and share a small
common core (`probe_common.py` / `probe_common.sh`). `just kind-probe-scripts`
publishes those files as the `quackgis-probe-scripts` ConfigMap consumed by the
QGIS, OGR, GeoServer, and demo Jobs.

`just seed-kind-demo` refreshes stable `public.demo_points` and
`public.demo_polygons` layers in an existing deployment. `just demo-kind` wraps
cluster readiness, deployment refresh, and that seed job for the five-minute path.

`just kind-compatibility` is the stable local/CI entrypoint: it refreshes the
Kind deployment with the fast probe build, then runs all maintained client jobs.
Without an activated shell, use `mise exec -- just kind-compatibility`.

`just kind-build-image` builds `quackgis-server` locally first and copies only
the release binary into a runtime image, so normal Cargo caches are reused. Use
`just kind-build-image-fast` / `just kind-refresh-fast` for manual probe triage;
they use Cargo's `probe` profile (release-like, but no
thin-LTO/single-codegen-unit) to cut rebuild latency. Use
`just kind-build-image-container` when you need a clean container-native build.

Run these recipes from an activated mise shell so Kind, kubectl, podman, Cargo,
and Just all come from the repo-pinned environment.

`just kind-probes` starts the maintained QGIS read, QGIS edit, OGR, and
GeoServer WFS/WMS/WFS-T Jobs together and waits once, which is the fastest full
client-probe loop after a refresh. Individual probe targets remain available for
focused triage.

`kind-qgis-probe` is the QGIS add-layer/read-feature gate.
`kind-qgis-edit-probe` opens a keyless spatial layer through the QGIS postgres
provider and exercises insert/update/delete/commit with `_quackgis_rowid`.
`kind-ogr-probe` uses GDAL's PostgreSQL driver (`ogrinfo`/`ogr2ogr`) to read a
WKB-backed table, append a GeoJSON layer with `PG_USE_COPY=NO` + `-addfields`,
and assert both SQL and GeoJSON read-back, including dynamically appended fields.
`kind-geoserver-probe` uses official GeoServer 3.0.0 plus pgjdbc to register a
PostGIS datastore, publish a WKB-backed layer, verify WFS GeoJSON feature count,
verify WMS returns a PNG, and exercise real WFS-T insert/update/delete.

`kind-osm-postgis-parity` is opt-in and not part of the default probe loop. It
starts `postgis-osm`, downloads a real Geofabrik OSM extract, loads points,
lines, and multipolygons into PostGIS, copies deterministic samples into
QuackGIS with `ogr2ogr`/`PG_USE_COPY=NO`, and compares GeoJSON exports plus SQL
samples from both databases. Use
`kind-postgis-osm-down` to remove the reference PostGIS deployment.

The `ducklake` PVC on the StatefulSet is intentionally shared by restarts of the
single QuackGIS pod. Future multi-server tests should switch the catalog/data
backend to a shared RWX/object-store setup before scaling replicas.
