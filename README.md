# QuackGIS

A PostGIS-compatible spatial database server in a single Rust binary:
PostgreSQL wire protocol via
[datafusion-postgres](https://github.com/datafusion-contrib/datafusion-postgres),
spatial execution via [Apache SedonaDB](https://github.com/apache/sedona-db),
storage via [DuckLake](https://ducklake.select)
([datafusion-ducklake](https://github.com/datafusion-contrib/datafusion-ducklake)).

No PostgreSQL. No DuckDB. The goal: spatial clients — **QGIS, GeoServer,
GDAL/OGR, psycopg** — connect and work without significant changes.

```text
QGIS / GeoServer (JDBC) / psql / OGR / psycopg
        │  pgwire
        ▼
quackgis server (one Rust binary)
├── datafusion-postgres   wire protocol · auth · TLS · pg_catalog
├── quackgis compat layer geometry OID/EWKB · geometry_columns ·
│                         spatial_ref_sys · client shims
├── SedonaDB              ST_* kernels · CRS · spatial joins (DataFusion)
└── datafusion-ducklake   DuckLake catalog + Parquet
        ▼
catalog DB (dev: SQLite, prod: PostgreSQL target) + Parquet (dev: local files, prod: AWS S3 target)
```

## Status

**Redesign in progress.** The architecture above was adopted after v0.1
validated (then retired) a heavier stack: full PostgreSQL + vendored
pg_ducklake + a C geometry extension + a DuckDB spatial extension. The wire
adaptor approach replaces all four layers with DataFusion-native components.

Current milestone focus: **M3/M4 client compatibility**. Martin, QGIS read/edit,
OGR load/read, and GeoServer WFS/WMS smoke probes are green. See
[ROADMAP.md](./ROADMAP.md) for milestones and the risk register.

## Quick start (dev storage path)

```sh
mise install              # Rust, just, kind/kubectl/helm, cargo-nextest
eval "$(mise activate bash)" # optional: activate pinned tools/env for this shell
just setup                # also downloads Martin into .tmp/bin
just ref-init             # optional: clone all reference repos into .tmp/ref
just server               # runs on 127.0.0.1:5434 with .tmp/dev storage
psql -h 127.0.0.1 -p 5434 -U postgres
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));        -- POINT(1 2)
CREATE TABLE quackgis.main.parcels AS SELECT 1::INT AS id; -- DuckLake + Parquet
INSERT INTO quackgis.main.parcels SELECT 2::INT AS id;
SELECT postgis_version();                                -- 3.4 QUACKGIS
```

## Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) — layer model, geometry over the wire,
  trust boundaries, what changed from v0.1.
- [ROADMAP.md](./ROADMAP.md) — milestones M0–M7, success metrics, risks.
- [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) — client compatibility
  targets and known limitations.
- [docs/OPERATIONS.md](./docs/OPERATIONS.md) — current local + Kind client-probe
  workflow for the single Rust pgwire binary.
- [docs/OSM_POSTGIS_PARITY.md](./docs/OSM_POSTGIS_PARITY.md) — real OSM data
  side-by-side PostGIS parity roadmap and copy/sync recipes.
- [CHANGELOG.md](./CHANGELOG.md) — history, including the retired v0.1 facade.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — contribution guide.

## Development

Use `mise` for pinned tools/env and `just` for repo workflows. For an
interactive shell, activate mise once, then run recipes directly. For the
guided path, start with [docs/QUICKSTART.md](./docs/QUICKSTART.md).

```sh
eval "$(mise activate bash)"
just --list                    # common entrypoints
just doctor                    # verify pinned local dev tools are available
just smoke                     # smallest pgwire + spatial query smoke test
just demo-kind                 # 5-minute Kind demo; see docs/QUICKSTART.md
just ci                        # same fast gate used by GitHub Actions
just build                     # server binary
just test                      # unit + wire integration tests
just test-fast                 # non-ignored QuackGIS regression loop only
just check                     # fmt + clippy + tests
just check-fast                # fmt + clippy + focused regression loop
just martin-sql                # Martin-generated SQL compatibility gate
just martin-e2e                # opt-in real Martin binary E2E
just kind-refresh              # host-cached build/load/deploy into Kind
just kind-refresh-fast         # faster no-LTO probe build/load/deploy loop
just kind-ready                # validate podman + create/reuse local Kind cluster
just seed-kind-demo            # seed stable public.demo_* layers in an existing cluster
just kind-probes               # QGIS read/edit + OGR + GeoServer WFS/WMS/WFS-T jobs
just kind-qgis-probe           # headless PyQGIS add-layer/read-feature gate
just kind-qgis-edit-probe      # headless PyQGIS insert/update/delete/save gate
just kind-ogr-probe            # GDAL/OGR PostgreSQL-driver load/read gate
just kind-geoserver-probe      # GeoServer 3.0.0 datastore + WFS/WMS/WFS-T gate
just kind-compatibility        # build/deploy + QGIS/OGR/GeoServer compatibility probes
just kind-osm-postgis-parity   # opt-in real OSM PostGIS -> QuackGIS parity
```

For one-off commands without shell activation, keep the same recipes and let mise
inject the pinned environment:

```sh
mise exec -- just ci
mise exec -- just kind-compatibility
```

Reference/source trees for client-trace work live outside the build graph under
ignored `.tmp/ref/*` (submodule-init equivalent): `just ref-init` materializes
the QuackGIS forks plus Martin, QGIS, GeoServer, GDAL/OGR (`ogr2ogr`), PostGIS,
DuckDB/DuckLake/pg_ducklake, and SQLite.

The current stack is intentionally zero-native-dependency for QuackGIS itself:
no libgeos/libproj/libgdal. Client/test tools such as Martin, QGIS, GeoServer,
and KinD are managed via `mise.toml` environment/tool pins plus Justfile
recipes.

Pushes to `main` and pull requests run the mise-backed fast Rust gate. The
scheduled/manual compatibility workflow builds the Kind image, runs QGIS
read/edit, OGR, GeoServer, and optionally real OSM PostGIS parity probes, then
uploads probe logs as a compatibility report artifact.

Upstreams are consumed through fork branches when needed. DuckLake storage is a **priority validated path**, not a placeholder: dev = SQLite catalog + local Parquet files; production target = PostgreSQL catalog + AWS S3 Parquet. Extending datafusion-ducklake to meet QuackGIS storage requirements (SQL DDL routing, UPDATE/DELETE, pruning, PostgreSQL/S3 hardening) is explicitly in scope, while staying forward-compatible with the official DuckLake 1.0+ spec.

Licensed under the [Apache License 2.0](./LICENSE).
