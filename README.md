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

Current milestone focus: **M3/M4 client compatibility**. Martin, QGIS read, and
OGR read probes are green; QGIS/OGR write paths and GeoServer remain trace-driven
targets. See [ROADMAP.md](./ROADMAP.md) for milestones and the risk register.

## Quick start (dev storage path)

```sh
mise install              # Rust, just, kind/kubectl/helm, cargo-nextest
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
- [CHANGELOG.md](./CHANGELOG.md) — history, including the retired v0.1 facade.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — contribution guide.

## Development

```sh
just --list                    # common entrypoints
just build                     # server binary
just test                      # unit + wire integration tests
just check                     # fmt + clippy + tests
just martin-sql                # Martin-generated SQL compatibility gate
just martin-e2e                # opt-in real Martin binary E2E
just kind-refresh              # host-cached build/load/deploy into Kind
just kind-qgis-probe           # headless PyQGIS add-layer/read-feature gate
just kind-ogr-probe            # GDAL/OGR PostgreSQL-driver read-back gate

Reference/source trees for client-trace work live outside the build graph under
ignored `.tmp/ref/*` (submodule-init equivalent): `just ref-init` materializes
the QuackGIS forks plus Martin, QGIS, GeoServer, GDAL/OGR (`ogr2ogr`), PostGIS,
DuckDB/DuckLake/pg_ducklake, and SQLite.
```

The current stack is intentionally zero-native-dependency for QuackGIS itself:
no libgeos/libproj/libgdal. Client/test tools such as Martin, QGIS, GeoServer,
and KinD are managed via `mise.toml` environment/tool pins plus Justfile
recipes.

Pushes to `main` and `v*` tags run the mise-backed CI artifact workflow. It
validates the pinned dev toolchain, uploads Linux x86_64 binary tarballs, pushes
the runtime image to GHCR on non-PR refs, and attaches binaries to GitHub
Releases for version tags.

Upstreams are consumed through fork branches when needed. DuckLake storage is a **priority validated path**, not a placeholder: dev = SQLite catalog + local Parquet files; production target = PostgreSQL catalog + AWS S3 Parquet. Extending datafusion-ducklake to meet QuackGIS storage requirements (SQL DDL routing, UPDATE/DELETE, pruning, PostgreSQL/S3 hardening) is explicitly in scope, while staying forward-compatible with the official DuckLake 1.0+ spec.

Licensed under the [Apache License 2.0](./LICENSE).
