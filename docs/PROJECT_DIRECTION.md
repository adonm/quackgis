# Project direction

## Product thesis

QuackGIS lets PostgreSQL-oriented GIS tools read DuckDB Spatial and DuckLake data
without reimplementing PostgreSQL. Real PostgreSQL/PostGIS is the compatibility
façade; DuckDB remains the analytical/spatial worker; iroh supplies transport;
small existing HTTP servers supply cacheable web formats.

The product wins by deleting compatibility code, not by owning another database
server.

## Target users

- QGIS users opening read-only PostgreSQL layers.
- External GeoServer deployments consuming the PostgreSQL façade.
- GDAL/OGR and `ogr2ogr` reading PostgreSQL or OGC API Features.
- Web maps consuming Martin TileJSON/MVT or immutable PMTiles.
- Applications consuming bounded OGC API Features through `pg_featureserv`.

## First release outcome

One edge deployment connects to one DuckDB/DuckLake worker and proves:

1. real PostgreSQL 18/PostGIS 3.6 client behavior;
2. Quack transport through a localhost iroh endpoint;
3. native PostGIS geometry with authoritative SRID/type metadata;
4. remote projection, scalar restriction, limit, and viewport bbox pushdown;
5. read-only QGIS and GDAL/OGR workflows on a representative layer;
6. Caddy-routed Martin and `pg_featureserv` OGC API Features endpoints; and
7. immutable revision URLs with deterministic cache headers.

The first release may use one manually published dataset and one worker. Correctness and a
bounded cold viewport matter more than automated administration.

## Explicit non-goals

- No custom pgwire server, PostgreSQL catalog emulation, or PostGIS function
  emulation.
- No bundled GeoServer or MapServer.
- No WMS, WFS, WCS, WFS-T, or general map-rendering platform.
- No synchronous general-purpose write path through foreign tables.
- No distributed SQL router, worker scheduler, consensus system, or shared cache.
- No broad translation of arbitrary PostGIS predicates.
- No promise that `duckdb_fdw` main or Quack beta is production-ready before the
  repository's own gates pass.

## Product surfaces

| Surface | First-release owner |
|---|---|
| PostgreSQL/PostGIS | PostgreSQL 18 + PostGIS 3.6 |
| DuckDB bridge | narrowly patched/pinned `duckdb_fdw` if upstream is insufficient |
| Remote worker protocol | DuckDB Quack through an iroh-local tunnel |
| Vector tiles and TileJSON | Martin |
| Static tile archive | PMTiles through Martin or Caddy |
| Feature HTTP API | required `pg_featureserv`, horizontally replicable behind an HTTP load balancer |
| Public TLS and routing | Caddy |

## Decision rules

1. Use an existing released component before adding QuackGIS code.
2. Patch `duckdb_fdw` only for a measured client contract: geometry conversion,
   CRS metadata, bbox translation, read-only enforcement, or resource safety.
3. Keep patches narrow and upstreamable; pin the exact source commit while 2.x
   remains unreleased.
4. Reject local execution that scales with complete layer size for a bounded
   viewport request.
5. Prefer immutable artifacts and revision URLs over cache invalidation.
6. Keep iroh and remote data credentials out of PostgreSQL client input.
7. Add a service only when a named client requires its protocol.

## Legacy implementation

The prior Rust pgwire/control edge is preserved at Git commit `17a7710` and on
the `main` branch as of this reset. Its tests and documentation remain useful as
client traces and semantic oracles, but its protocol/catalog implementation is
not carried into the first release unless a focused fixture can test the real
PostgreSQL façade.
