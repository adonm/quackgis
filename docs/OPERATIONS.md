# Operations

QuackGIS v0.2 is a single Rust pgwire server. It does **not** run PostgreSQL,
DuckDB, pg_ducklake, or C extensions in-process. DuckLake metadata is currently
SQLite-backed in development and table data is local Parquet; production
PostgreSQL-catalog/S3 hardening remains a roadmap item.

## Local development

```sh
mise install
eval "$(mise activate bash)"
just build
just server
```

Assume an activated mise shell for the commands below; this keeps the pinned
Rust, container, Kubernetes, and probe-tool environment on `PATH` without
wrapping every command in `mise exec`.

The default local server listens on `127.0.0.1:5434` and uses:

| Variable | Default | Purpose |
|---|---|---|
| `QUACKGIS_HOST` | `127.0.0.1` | bind host |
| `QUACKGIS_PORT` | `5434` | pgwire port |
| `QUACKGIS_CATALOG_PATH` | `.tmp/dev/quackgis.db` | DuckLake SQLite catalog |
| `QUACKGIS_DATA_PATH` | `.tmp/dev/data` | Parquet data directory |
| `QUACKGIS_LOG` | `info` | Rust log filter |

Dev auth is intentionally minimal: connect as user `postgres` to database
`quackgis` with no password unless a future auth layer is enabled.

## Kind client probes

Containerized client tests should run inside Kind, not via host networking. This
gives stable service DNS, consistent auth, and room to add multi-pod/multi-client
DuckLake tests later.

```sh
eval "$(mise activate bash)"
just kind-up
just kind-refresh-fast   # probe loop: no release thin-LTO
just kind-probes         # QGIS read/edit + OGR + GeoServer in one wait
```

`just kind-refresh` builds the release binary locally with Cargo's normal
`target/` cache, copies it into a tiny runtime image, loads that image into Kind,
then restarts the StatefulSet so the fixed dev tag is picked up. For iterative
probe triage, `just kind-refresh-fast` uses Cargo's `probe` profile (release-like
but no thin-LTO/single-codegen-unit) to reduce rebuild latency. Use
`just kind-build-image-container` for a slower clean build inside the container
image.

`just kind-probes` starts the maintained QGIS read, QGIS edit, OGR, and
GeoServer Jobs together and waits once. Individual `kind-qgis-probe`,
`kind-qgis-edit-probe`, `kind-ogr-probe`, and `kind-geoserver-probe` targets
remain available for focused reruns.

The QGIS probe is a read-path gate. Current expected output includes:

```text
valid True
feature_count 2
fields ['id', 'name']
features_read 2
```

The QGIS edit probe opens a keyless spatial table through the postgres provider,
uses `_quackgis_rowid` as feature identity, and commits insert/update/delete
edits. Current expected output ends with:

```text
after_insert ... 'inserted' ... 'Point (1 1)' ...
after_update ... 'updated' ... 'Point (2 2)' ...
after_delete ... 'updated' ... 'Point (2 2)' ...
edit_ok True
```

The OGR probe uses GDAL's PostgreSQL driver to read a WKB-backed table, append a
GeoJSON layer with `PG_USE_COPY=NO` + `-addfields`, and export both paths to
GeoJSON. Current expected output includes:

```text
feature_count 2
names ['one', 'origin']
geometry_types ['Point', 'Point']
loaded_rows [('load-a', 'client', 'POINT(2 2)'), ('load-b', 'client', 'POINT(3 3)')]
load_feature_count 2
load_names ['load-a', 'load-b']
load_geometry_types ['Point', 'Point']
```

The GeoServer probe uses official `docker.osgeo.org/geoserver:3.0.0`, supplies a
pgjdbc jar, registers QuackGIS as a PostGIS datastore, publishes a WKB-backed
layer, and exercises WFS GeoJSON plus WMS PNG rendering. Current expected output
includes:

```text
wfs_point_count 2
wms_png_header 89504e470d0a1a0a
geoserver_probe_ok True
```

In-cluster clients connect to:

```text
host: quackgis.quackgis.svc.cluster.local
port: 5434
user: postgres
database: quackgis
password: <empty>
```

Relevant files:

| Path | Purpose |
|---|---|
| `deploy/Containerfile.runtime` | runtime-only image used by the cached host-build Kind path |
| `deploy/Containerfile` | clean container-native fallback image for Kind probes |
| `deploy/kind/cluster.yaml` | Kind cluster config |
| `deploy/kind/quackgis.yaml` | QuackGIS StatefulSet + Service |
| `deploy/kind/qgis-probe.yaml` | headless PyQGIS add-layer probe Job |
| `deploy/kind/qgis-edit-probe.yaml` | headless PyQGIS edit/save probe Job |
| `deploy/kind/ogr-probe.yaml` | GDAL/OGR PostgreSQL-driver load/read probe Job |
| `deploy/kind/geoserver-probe.yaml` | official GeoServer 3.0.0 datastore + WFS/WMS probe Job |
| `deploy/kind/postgis-osm.yaml` | opt-in PostGIS reference deployment for real OSM parity |
| `deploy/kind/osm-postgis-parity-probe.yaml` | opt-in real OSM PostGIS → QuackGIS copy/read parity Job |

### Opt-in real OSM PostGIS parity

The real-data parity track is intentionally outside `just kind-probes` because it
pulls a PostGIS image and downloads a live OSM extract. It uses Geofabrik Monaco
by default, loads real OSM points into PostGIS, copies a deterministic named
Point sample into QuackGIS with `ogr2ogr`, and compares GeoJSON exports plus SQL
samples from both databases.

```sh
eval "$(mise activate bash)"
just kind-refresh-fast
just kind-osm-postgis-parity
```

Current expected output includes:

```text
postgis_osm_named_points_count 50
quackgis_osm_named_points_count 50
osm_postgis_to_quackgis_copy_ok True
```

The gate asserts stable IDs, `osm_id`, UTF-8 names, geometry type, count, and
bbox. It prints PostGIS and QuackGIS SQL samples as evidence for text and
attribute parity.

Useful overrides:

```sh
OSM_EXTRACT_URL=https://download.geofabrik.de/europe/andorra-latest.osm.pbf \
OSM_POINT_LIMIT=100 \
just kind-osm-postgis-parity
```

Stop the reference PostGIS deployment when finished:

```sh
just kind-postgis-osm-down
```

See [OSM_POSTGIS_PARITY.md](./OSM_POSTGIS_PARITY.md) for the long roadmap and
copy/sync recipes.

## CI artifacts

GitHub Actions uses `mise.toml` as the CI toolchain source of truth. The
`CI artifacts` workflow runs formatting, tests, clippy, and release builds with
mise. For local work, prefer an activated mise shell over repeated `mise exec`
wrappers.

- Pushes to `main` and version tags publish a runtime image to
  `ghcr.io/adonm/quackgis` with branch/tag/SHA tags.
- Every workflow run uploads a Linux x86_64 binary tarball as a CI artifact.
- Version tags matching `v*` also attach that tarball and its `.sha256` file to
  the corresponding GitHub Release.

## Persistence model

The Kind StatefulSet mounts one `ducklake` PVC at `/var/lib/quackgis` containing:

- `quackgis.db` — DuckLake SQLite catalog
- `data/` — Parquet data files

This is suitable for single-pod restart/persistence smoke tests. Multi-server
tests must move to a shared catalog/data backend (for example PostgreSQL catalog
+ object-store data) before scaling replicas.

## Reference source checkouts

`just ref-init` materializes source trees under ignored `.tmp/ref/*` for fork
work and client-trace research. This is intentionally submodule-like but outside
the build graph: Cargo continues to consume canonical git dependencies pinned by
`Cargo.lock`.

## Removed stale v0.1 deploy assets

The old PostgreSQL-container Helm chart, `container/Dockerfile*`, BuildKit
scripts, `pg_isready` probes, and DuckDB/pg_ducklake environment variables were
removed from the current deploy path. Git history retains them for archaeology;
new deployment work should target the single `quackgis-server` binary.
