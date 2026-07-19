# First-release roadmap

The roadmap is ordered by uncertainty. Packaging and endpoint polish do not
precede proof that a cold spatial viewport is bounded at the worker.

## P0 — reset and preserve

**Outcome:** the new branch is truthful and the previous implementation remains
recoverable.

- [x] Commit and push the prior main worktree (`17a7710`).
- [x] Create and rename the architecture-reset branch to `quackgis-next`.
- [x] Replace top-level direction, architecture, and roadmap documents.
- [x] Exclude bundled GeoServer and MapServer; select Caddy.
- [ ] Move retained v1 trace fixtures behind an explicit `legacy` boundary before
  deleting old runtime crates or scripts.

## P1 — Quack FDW feasibility

**Outcome:** PostgreSQL reads one remote DuckDB table over Quack.

Deliver:

- reproducible PostgreSQL 18/PostGIS 3.6 image;
- exact `duckdb_fdw` source commit and DuckDB 1.5.x pin;
- one DuckDB Quack worker with a deterministic fixture;
- scalar projection/filter/limit smoke tests; and
- `EXPLAIN (VERBOSE)` capture of generated remote SQL.

Exit gates:

- all selected binaries and source commits are recorded;
- a PostgreSQL read returns exact remote rows;
- `WHERE id = ...` and `LIMIT` appear in remote SQL;
- the PostgreSQL backend does not open worker files or object storage; and
- reconnect and two concurrent readers pass.

The `deploy/quackgis/` stack now passes these fixture-scale gates, including two
concurrent readers and reconnect through iroh. Exact source/image/artifact pins
are tracked, and extension installation occurs at image build rather than
runtime.

## P2 — geometry and viewport critical path

**Outcome:** QGIS-style viewport reads are native and bounded.

Deliver:

- DuckDB geometry to PostgreSQL PostGIS geometry through WKB/EWKB;
- fixed layer SRID/family/dimension configuration;
- `IMPORT FOREIGN SCHEMA` mapping or deterministic generated foreign-table DDL;
- translation of `geom && ST_MakeEnvelope(...)` to a worker-side bbox predicate;
- optional numeric bbox-column translation and local exact recheck; and
- explain/row-count/bytes-read evidence on selective and nonselective probes.

Exit gates:

- PostgreSQL reports `geometry(<family>, <srid>)`, not `text` or `bytea`;
- empty, NULL, malformed, and wrong-SRID cases fail or round-trip explicitly;
- remote SQL contains the viewport candidate filter;
- a selective viewport does not transfer the complete fixture; and
- the plan remains bounded at 1M representative features.

If this requires broad PostgreSQL/PostGIS emulation rather than a narrow FDW
patch, stop and reevaluate `ogr_fdw` instead of rebuilding the old server.

Fixture-scale native geometry, NULL/error handling, QGIS/GDAL/Martin/featureserv
bbox translation, exact local recheck, and bounded extent metadata now pass.
Representative 1M-feature row-group/scan-byte evidence remains the P2 exit gate.

## P3 — named client proof

**Outcome:** the common live feature contract works with real clients.

- QGIS direct PostgreSQL layer discovery, extent, pan/zoom, identify, and subset.
- GDAL/OGR discovery and bbox-filtered read.
- External GeoServer PostGIS-store discovery and bounded read.
- Read-only role denial for INSERT/UPDATE/DELETE and unsafe helper functions.

QuackGIS supports GeoServer as an external client; it does not bundle GeoServer.
QGIS 3.44.11 and GDAL/OGR viewport workflows pass in the development stack;
external GeoServer qualification remains open.

## P4 — cacheable edge

**Outcome:** one Caddy hostname exposes bounded or immutable HTTP reads.

- Martin TileJSON/MVT from the proven PostgreSQL view.
- Immutable PMTiles as the preferred stable-map path.
- Required `pg_featureserv` OGC API Features from the same read-only view, with
  stateless replicas suitable for a future Kubernetes Service.
- Caddy TLS, routing, compression, static range requests, and revision cache
  headers.
- Short-lived or redirecting `latest`; year-long immutable revision URLs.

Exit gates:

- cold dynamic tiles inherit the P2 bounded viewport plan;
- PMTiles requests require no PostgreSQL query;
- authenticated responses are not shared-cacheable by default; and
- QGIS and a browser client consume advertised endpoints.

The development stack now serves bounded dynamic MVT/TileJSON, required OGC API
Features, and one immutable PMTiles fixture with the intended cache policy.
Production TLS, browser integration, and real revision publication remain open.

## P5 — QuackGIS package

**Outcome:** one operator command starts a constrained read-only edge.

- Compose/Podman bundle with separate PostgreSQL, Martin, `pg_featureserv`,
  Caddy, and iroh processes.
- No runtime extension installation.
- Owner-only secrets and read-only database roles.
- Health/readiness checks, resource limits, backup of edge configuration, and a
  documented worker-unavailable failure mode.
- Exact image/source digests and licenses.

## After the first release

- asynchronous upload/validate/load/publish workflow;
- automatic layer/endpoint registration;
- multiple workers and assignment policy;
- additional PostGIS predicate translation based on measured traces; and
- optional Caddy shared-cache module after cache-key/security tests.

The isolated `quackgis-multi` profile now proves the narrower feasibility case:
two statically assigned, read-only workers and two edges can serve one frozen
local DuckLake snapshot to four concurrent clients. Dynamic assignment,
concurrent publication, remote shared catalogs/object storage, refresh, and
failover remain post-first-release work.
