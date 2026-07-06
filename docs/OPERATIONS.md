# Operations

QuackGIS v0.2 is a single Rust pgwire server. It does **not** run PostgreSQL,
DuckDB, pg_ducklake, or C extensions in-process. DuckLake metadata is currently
SQLite-backed in development and table data is local Parquet; production
PostgreSQL-catalog/S3 hardening remains a roadmap item.

## Local development

```sh
mise install
just build
just server
```

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
just kind-up
just kind-refresh
just kind-qgis-probe
```

`just kind-refresh` uses the fast dev path: build `quackgis-server` locally with
Cargo's normal `target/` cache, copy the release binary into a tiny runtime image,
load that image into Kind, then restart the StatefulSet so the fixed dev tag is
picked up. Use `just kind-build-image-container` for a slower clean build inside
the container image.

The QGIS probe is a read-path gate. Current expected output includes:

```text
valid True
feature_count 2
fields ['id', 'name']
features_read 2
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
