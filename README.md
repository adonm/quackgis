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
catalog DB (SQLite/PG) + Parquet on file/S3
```

## Status

**Redesign in progress.** The architecture above was adopted after v0.1
validated (then retired) a heavier stack: full PostgreSQL + vendored
pg_ducklake + a C geometry extension + a DuckDB spatial extension. The wire
adaptor approach replaces all four layers with DataFusion-native components.

Current milestone: **M0 — skeleton server** (SedonaDB context served over
pgwire, psql smoke test). See [ROADMAP.md](./ROADMAP.md) for milestones and
the risk register.

## Target quick start (post-M1)

```sh
docker run -e QUACKGIS_PASSWORD=quackgis -p 5432:5432 quackgis:dev
psql postgres://postgres:quackgis@localhost:5432/quackgis
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));        -- POINT(1 2)
CREATE TABLE parcels (id int, geom geometry);            -- DuckLake + Parquet
SELECT postgis_version();                                -- 3.4 QUACKGIS
```

## Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) — layer model, geometry over the wire,
  trust boundaries, what changed from v0.1.
- [ROADMAP.md](./ROADMAP.md) — milestones M0–M7, success metrics, risks.
- [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) — client compatibility
  targets and known limitations.
- [docs/OPERATIONS.md](./docs/OPERATIONS.md) — deploy/backup recipes
  (v0.1-era; refreshed at M6).
- [CHANGELOG.md](./CHANGELOG.md) — history, including the retired v0.1 facade.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — contribution guide.

## Development

```sh
cargo build --release          # server binary at target/release/quackgis-server
cargo test                     # unit + wire integration tests
cargo run --release -- --host 0.0.0.0 --port 5434
```

M0 dev note: `sedonadb`'s default feature set builds against GEOS, so the
host needs `libgeos` installed. On Linux: `apt install libgeos-dev`; on macOS
`brew install geos`. CI installs it automatically.

Upstreams are pinned forks: several needed capabilities don't exist upstream
yet (DuckLake UPDATE/DELETE + pruning, SQL cursors, deep pg_catalog — see the
gap ledger in [ROADMAP.md](./ROADMAP.md)), so we build them in our forks and
upstream opportunistically. This repo owns the PostGIS compatibility surface
and the glue.

Licensed under the [Apache License 2.0](./LICENSE).
