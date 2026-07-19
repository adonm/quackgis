# Contributing

This branch is implementing the real-PostgreSQL QuackGIS direction described in
[docs/PROJECT_DIRECTION.md](docs/PROJECT_DIRECTION.md).

## Rules

- Do not extend the owned Rust pgwire/PostGIS-emulation implementation.
- Use PostgreSQL/PostGIS for client compatibility and DuckDB for remote execution.
- Keep `duckdb_fdw` changes narrow, measured, and upstreamable.
- Every compatibility change needs a captured query from QGIS, external
  GeoServer, GDAL/OGR, Martin, or `pg_featureserv`.
- A viewport or tile query must have a worker-side candidate restriction before
  local exact recheck.
- Geometry crosses engine boundaries as WKB/EWKB with explicit CRS/type metadata;
  do not introduce WKT as a release data path.
- Services are read-only and least-privileged by default.
- Pin source commits, images, and DuckDB extensions before making release claims.
- Do not add GeoServer or MapServer to the bundle. Caddy is the HTTP proxy.
- Prefer immutable revision URLs over mutable cache invalidation.

## Current loop

```sh
just quackgis-up
just quackgis-smoke
just quackgis-plan
just quackgis-down
```

`just quackgis-plan` currently records the missing spatial pushdown rather than
claiming success. The P2 gate in [ROADMAP.md](ROADMAP.md) closes only when native
PostGIS geometry and remote bbox filtering pass.
