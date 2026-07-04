# QuackGIS

A thin PostgreSQL/PostGIS-compatible facade container backed by DuckDB
execution, the `sedonadb` spatial extension, and DuckLake storage.

Clients connect with normal PostgreSQL tooling (`psql`, JDBC, psycopg, BI
tools). PostgreSQL provides pgwire, sessions, auth, and catalog behavior.
DuckDB executes analytical queries. `sedonadb` provides 180+ spatial `ST_*`
functions. DuckLake stores table data in Parquet.

```text
psql / JDBC / psycopg / BI tools
        │  pgwire
        ▼
PostgreSQL + pg_ducklake (table AM)
        │
        ▼
DuckDB + sedonadb (spatial execution)
        │
        ▼
DuckLake (Parquet / object storage)
```

## Quick start

```sh
docker build -t quackgis:dev -f container/Dockerfile .
docker run -e POSTGRES_PASSWORD=quackgis -p 5432:5432 quackgis:dev
psql postgres://postgres:quackgis@localhost:5432/postgres
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));        -- POINT(1 2)
SELECT ST_Area(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'));  -- 16
SELECT postgis_version();                                -- 3.4 QUACKGIS
```

## Status

Code-complete through M11 but **not yet validated end-to-end**. The Docker
image has a known build blocker (missing `libgeos-dev`) and several unproven
assumptions (see [ROADMAP.md](./ROADMAP.md)). Validation is the current focus.

## Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) — layer model, trust boundaries.
- [ROADMAP.md](./ROADMAP.md) — current status, known blockers, what's next.
- [CHANGELOG.md](./CHANGELOG.md) — release history.
- [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) — supported configs and
  known limitations.
- [docs/OPERATIONS.md](./docs/OPERATIONS.md) — deploy, backup, upgrade,
  migration, DuckLake layout recipes.
- [COMPATIBILITY.md](./COMPATIBILITY.md) — engine-level function catalog
  (auto-generated).
- [CONTRIBUTING.md](./CONTRIBUTING.md) — engine and facade contribution guide.

## What this repo contains

**Engine** (pre-existing, tested):
- DuckDB `sedonadb` spatial extension: 254 functions over WKB/EWKB geometry.
- Literal Apache SedonaDB bridge, GEOS, PROJ, GDAL, GeoRust backends.
- PostGIS SQL rewriter (`sedonadb_rewrite_postgis`, `sedonadb-migrate`).
- DuckLake spatial layout functions (`st_quadkey`, `st_hilbert`, etc.).
- 860+ SQL regression tests.

**Facade container** (M0–M11, unvalidated):
- Multi-stage Dockerfile (pg_ducklake base + sedonadb + spatial libs).
- 8 init SQL scripts: DuckDB config, bridge table, diagnostics, geometry
  DOMAIN + operators, 50 manual stubs + 112 generated stubs, aggregates,
  DuckLake layout helpers.
- Test suite: smoke, PostGIS compat, DuckLake persistence, psycopg, backup.
- Helm chart + K8s manifests + KinD test.
- CI/CD: engine + container + facade + release automation.

## Development

```sh
# Engine
cargo test --lib
./tests/run_sql.sh

# Container (after fixing build blocker)
./container/build.sh
./container/smoke-test.sh
```

Licensed under the [Apache License 2.0](./LICENSE).
