# Architecture

QuackGIS uses real PostgreSQL/PostGIS as its compatibility boundary and keeps
DuckDB execution outside PostgreSQL storage. It does not implement pgwire,
PostgreSQL catalogs, or PostGIS functions.

## Runtime topology

```text
                         QuackGIS edge
┌─────────────────────────────────────────────────────────────────┐
│                                                                 │
│  PostgreSQL 18 + PostGIS 3.6                                    │
│    read-only roles · stable schemas/views · geometry typmods    │
│           │                                                     │
│           └── duckdb_fdw ── embedded DuckDB Quack client        │
│                                      │ localhost                │
│                                      ▼                          │
│                                iroh tunnel/proxy                 │
│                                                                 │
│  Caddy :443                                                     │
│    /tiles/*    ──► Martin                                       │
│    /features/* ──► pg_featureserv                               │
│    /archives/* ──► immutable static PMTiles                     │
│                                                                 │
└──────────────────────────────────────┬──────────────────────────┘
                                       │ iroh
                                       ▼
                            DuckDB Spatial / DuckLake
```

PostgreSQL may be exposed separately on port 5432. Martin,
`pg_featureserv`, PostgreSQL administration, and the local Quack/iroh endpoint
remain on the private deployment network.

## Component ownership

| Component | Owns | Does not own |
|---|---|---|
| PostgreSQL/PostGIS | pgwire, catalogs, roles, PostGIS types/functions, stable client-facing views | DuckLake storage or remote query execution |
| `duckdb_fdw` | PostgreSQL planner integration, SQL deparsing, type conversion, Quack attachment | PostgreSQL compatibility emulation or dataset publication |
| local DuckDB client | loading the Quack extension and attaching the remote catalog | durable user data or heavy execution |
| iroh sidecar | authenticated transport from a localhost endpoint to a worker | SQL parsing, authorization, routing, or caching |
| DuckDB/DuckLake worker | analytical/spatial execution, snapshots, Parquet, staged publication | public pgwire or HTTP compatibility |
| Martin | MVT, TileJSON, PMTiles/MBTiles, styles and map resources | feature-service semantics or storage authority |
| `pg_featureserv` | OGC API Features over read-only PostGIS views | tiles, writes, or authorization authority |
| Caddy | TLS, routing, compression, cache headers, static immutable files | database authorization or snapshot identity |

Services are distributed as one Compose/Podman deployment but remain separate
containers and processes. GeoServer and MapServer are intentionally absent.

## Read paths

### Features

```text
client SQL / OGC bbox
  → PostgreSQL planner
  → duckdb_fdw translation
  → DuckDB Quack client
  → remote DuckDB query
  → WKB/EWKB result
  → PostGIS geometry
```

The first release requires projection and restrictions to reach the remote query. A local
PostGIS recheck is acceptable only after a conservative remote candidate filter.
Fetching the complete remote layer for a viewport is a release blocker.

`pg_featureserv` is a required first-release surface rather than an add-on. It
provides a stateless `/features/*` HTTP boundary that can later run as multiple
replicas behind a Kubernetes Service while Caddy retains one stable public route.
Replicas may scale connections and roll independently, but they are enabled only
after the same worker-side bbox gate passes; horizontal scaling must not multiply
an unbounded cold query.

### Tiles

Stable maps should be published as immutable PMTiles and served without a
database query. Dynamic Martin tiles may query PostGIS only after the same bbox
pushdown gate passes. Cache hits must not be used to hide an unbounded cold path.

## Geometry contract

Every published layer has:

- exactly one primary geometry column;
- a fixed geometry family and dimensions for the published revision;
- an explicit authoritative SRID;
- a stable feature identifier;
- finite extent metadata; and
- optional numeric bbox columns with useful remote statistics.

Geometry crosses the FDW boundary as WKB/EWKB, not lossy WKT. PostgreSQL exposes
a native `geometry(<type>, <srid>)` column. A layer with missing or conflicting
CRS/type metadata fails publication.

The minimum translated spatial shape is the expression used by QGIS and tile
servers:

```sql
geom && ST_MakeEnvelope($xmin, $ymin, $xmax, $ymax, $srid)
```

`EXPLAIN (VERBOSE)` must show a corresponding remote bbox restriction. Exact
`ST_Intersects` pushdown is useful but not required when PostgreSQL performs a
bounded exact recheck.

## Cache contract

Published URLs include an immutable dataset revision:

```text
/tiles/{dataset}/{revision}/{z}/{x}/{y}.pbf
/archives/{dataset}/{revision}.pmtiles
/features/{dataset}/{revision}/...
```

Immutable responses receive:

```http
Cache-Control: public, max-age=31536000, immutable
ETag: "{content-or-revision-hash}"
```

`latest` has a short TTL or redirects to a revision URL. Publishing creates a new
revision; it does not mutate cached content. Caddy's standard image provides TLS,
routing, headers, and static files but not a shared response cache. A pinned cache
module or CDN is optional after cache keys and authorization behavior are proven.

## Trust boundaries

- PostgreSQL, Martin, and `pg_featureserv` use separate read-only roles.
- Only allowlisted foreign servers, schemas, tables, and worker endpoints are
  created by an operator; clients cannot submit DuckDB connection or file paths.
- Quack tokens live in PostgreSQL user mappings or owner-only secret files, not
  foreign-server options visible to ordinary users.
- DuckDB extensions are pinned and preinstalled in release images. Runtime
  `INSTALL` is allowed only in the development proof of concept.
- The iroh sidecar exposes only a localhost endpoint and carries Quack traffic;
  it does not gain PostgreSQL or object-store credentials.
- Caddy does not cache authenticated or user-specific responses by default.

## Writes

The first release is read-only. Later writes use an asynchronous publication workflow:

```text
upload → validate → load/stage in DuckLake → verify → publish new revision
```

PostgreSQL foreign tables, WFS transactions, and arbitrary remote DML are not the
write API.
