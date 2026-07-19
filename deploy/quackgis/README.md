# QuackGIS proof of concept

This directory starts roadmap gate P1. It is a disposable development stack, not
a release deployment.

## What it proves

- PostgreSQL 18 and PostGIS 3.6 start from the upstream PostGIS image.
- `duckdb_fdw` is built from exact commit
  `9354241029df691b695f15428082b7c5cd81e2c7` against DuckDB 1.5.4.
- A tracked compatibility patch updates that exact source for PostgreSQL 18's
  foreign-path and EXPLAIN APIs and permits plain HTTP on this private
  development-only Quack network.
- PostgreSQL attaches to a DuckDB 1.5.4 Quack worker.
- Scalar remote rows are exposed through a read-only PostgreSQL role.
- A temporary WKT view demonstrates end-to-end wiring while making the missing
  WKB/EWKB and bbox pushdown work explicit.

The WKT bridge is intentionally named `features_unbounded`; it is not an accepted
release geometry path.

## Run

```sh
just quackgis-up
just quackgis-smoke
just quackgis-plan
just quackgis-down
```

`quackgis-up` needs outbound network access because both sides install DuckDB's Quack
extension from `core_nightly`. That behavior is forbidden in the release package.

## Expected current gap

`just quackgis-plan` prints PostgreSQL's plan for:

```sql
SELECT id
FROM public.features_unbounded
WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326);
```

The current FDW rejects PostGIS extension operators/functions for pushdown, so
the remote SQL has no spatial candidate. P2 replaces the WKT view with native
WKB/EWKB conversion and requires a remote bbox predicate.

## Development-only credentials

The checked-in passwords and Quack token are local disposable defaults. Do not
reuse this file as production secret management.
