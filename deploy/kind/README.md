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

`just probe-static-check` is the cheap pre-Kind gate. It byte-compiles all probe
Python, syntax-checks shared shell helpers, and scans `deploy/kind/*.yaml` for
basic Kubernetes document structure before a full image build or cluster probe.
The default `just ci` recipe runs this gate after the host-local smokes.
`just kind-compat-report` writes both the Markdown summary (`README.md`) and a
machine-readable `metrics.json` under `.tmp/compatibility` so scheduled QPS,
OLAP, writer, read, and OSM parity runs can be trended from uploaded artifacts.
In GitHub Actions, `metrics.json` is stamped with workflow/run/SHA metadata and
uploaded both inside the full report artifact and as a metrics-only artifact whose
name starts with `compatibility-metrics-<sha>-` or
`storage-metrics-kind-alpha-smoke-<sha>-`.

`just seed-kind-demo` refreshes stable `public.demo_points` and
`public.demo_polygons` layers in an existing deployment. `just demo-kind` wraps
cluster readiness, deployment refresh, and that seed job for the five-minute path.

`just kind-compatibility` is the stable local/CI entrypoint: it refreshes the
Kind deployment with the fast probe build, then runs all maintained client jobs.
Without an activated shell, use `mise exec -- just kind-compatibility`.

`just kind-lake-smoke` deploys the scaled-storage lake profile. It uses a
short-named QuackGIS Deployment/Service (`lake`), a PostgreSQL DuckLake catalog
(`pg`), and local S3-compatible object storage using `s3s-fs` (`s3`). The probe
creates a table, loads WKB points with PostgreSQL text `COPY FROM STDIN`, queries
via PostGIS-style functions, compacts the DuckLake table, and verifies unchanged
results.

`just kind-external-alpha-smoke` deploys `external-lake`, a second QuackGIS
service configured only from the `external-storage` Secret. Defaults point at the
Kind `pg`/`s3` emulators with a separate `quackgis_external` PostgreSQL
database; set `EXTERNAL_ALPHA_USE_KIND_EMULATORS=false` and `EXTERNAL_QUACKGIS_*`
variables to point at real external services. The probe
also verifies autocommit `DELETE` writes native DuckLake delete files for two
data files under one PostgreSQL metadata snapshot.

`just kind-qps-smoke` is the parallel-reader gate for that lake profile. It
scales `lake` to three pods, seeds a compacted LayoutBench-style table, runs 16
parallel pgwire reader connections for 240 selective spatial queries across
five window/predicate shapes, verifies backend spread with
`quackgis_instance_id()`, asserts file-group and bytes-scanned ceilings from
`EXPLAIN ANALYZE`, and reports QPS plus p50/p95/p99 latency. Use
`just kind-qps-mtls-smoke` for the same workload through a Linkerd-injected
client; that variant also asserts Linkerd outbound TCP/TLS open and byte metrics
for `lake` destination pods.

`just kind-qps-deep-smoke` is the opt-in larger version. Its defaults seed 1.08M
rows, use 32 reader connections, run 640 queries across four `lake` pods, and
assert Linkerd TCP/TLS metric deltas. Tune `QPS_DEEP_FACTOR`, `QPS_DEEP_WORKERS`,
`QPS_DEEP_QUERIES`, and `QPS_DEEP_DISK_BUDGET_GIB` for bigger hosts. The QPS
recipes set the server-side shared-catalog read refresh interval from
`QPS_SHARED_CATALOG_REFRESH_MS` (default 60000) for the stable read-only phase
and clear any stale global `QUACKGIS_TARGET_PARTITIONS` override. Static
hidden-bbox spatial reads automatically use
`QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS` (default 1) in a per-query session
clone, so whole-table seed/compaction keeps DataFusion's default parallelism.

`just kind-write-smoke` proves the Alpha writer path on the same PostgreSQL/S3
profile. It runs independent-table COPY writers, shared-table appends, verifies
counts/spatial reads after compaction, then forces a stale transactional replace
over a concurrent snapshot and expects `write_conflict conflict_observed=True`
before retrying against the newer snapshot.

`just kind-olap-smoke` seeds a columnar spatial asset table and verifies grouped
stats, hidden-bbox pruning, Parquet predicate/projection evidence, aggregate
execution evidence, a bytes-scanned ceiling, and exact SedonaDB recheck of
candidate groups. `just kind-alpha-smoke` bundles the maintained lake storage,
multi-pod, writer, QPS, and OLAP gates.

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
`kind-ogr-probe` uses GDAL's PostgreSQL driver (`ogrinfo`/`ogr2ogr`) with
`PG_USE_POSTGIS=NO` to read a WKB-backed table, append a GeoJSON layer with
`PG_USE_COPY=NO` + `-addfields`, and assert both SQL and GeoJSON read-back,
including dynamically appended fields.
`kind-geoserver-probe` uses official GeoServer 3.0.0 plus pgjdbc to register a
PostGIS datastore, publish a WKB-backed layer, verify WFS GeoJSON feature count,
verify WMS returns a PNG, and exercise real WFS-T insert/update/delete.

`kind-osm-postgis-parity` is opt-in and not part of the default probe loop. It
starts `postgis-osm`, downloads a real Geofabrik OSM extract, loads points,
lines, and multipolygons into PostGIS, copies deterministic samples into
QuackGIS with `ogr2ogr`/`PG_USE_COPY=NO`, and compares GeoJSON exports plus SQL
samples from both databases. Use
`kind-postgis-osm-down` to remove the reference PostGIS deployment.

The `ducklake` PVC on the default compatibility StatefulSet is intentionally
shared by restarts of the single QuackGIS pod. Use the lake Deployment for
multi-pod tests; it stores DuckLake metadata in PostgreSQL and Parquet objects in
the S3-compatible service instead of using the StatefulSet PVC.

The lake manifest is intentionally separate from the default compatibility
StatefulSet: `deploy/kind/lake.yaml` keeps the existing client-probe service
stable while exercising the shared PostgreSQL/S3 storage profile through
`lake.quackgis.svc.cluster.local:5434`.
