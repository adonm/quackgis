# API and SQL client probe contract

QGIS, GDAL/OGR, GeoServer, Martin, `psql`, and tokio-postgres/psycopg-style wire
paths are the maintained compatibility base. The next client expansion should be
trace-driven and cheap before it becomes another heavy Kind workflow.

This document defines the support bar for Python/API/BI-style clients.

## Candidate clients

| Client/profile | Target workflow | Support status |
|---|---|---|
| psycopg 3 | connect, parameterized WKB/EWKB, binary/text reads, COPY where applicable | profile surface covered by `just api-client-local-smoke` and `just kind-api-client-probe`; named psycopg dependency probe pending |
| SQLAlchemy Core | reflection enough for read queries and inserts into declared schemas | profile information_schema surface covered locally and in Kind; named SQLAlchemy dependency probe pending |
| GeoPandas `read_postgis` | read WKB-backed geometry into a GeoDataFrame, preserve CRS metadata when available | profile WKB row-query surface covered locally and in Kind; named GeoPandas dependency probe pending |
| pg_featureserv-like reader | table discovery, bbox filters, JSON/GeoJSON-shaped API reads over pgwire | profile bbox/filter surface covered locally and in Kind; real server pending |
| BI/SQL tools | simple/extended protocol SELECTs, projection/filter/grouped aggregates | profile grouped aggregate surface covered locally and in Kind; named BI client pending |
| MVT consumers through Martin | table discovery, non-empty tile, attribute tags | real Martin binary opt-in proves configured synthetic attributes; profile MVT query covers layer/key/value dictionaries locally and in Kind; real Martin OSM attribute matrix pending |

## Probe design rules

1. Start with a focused local/wire test that captures the catalog/protocol gap.
2. Add a containerized client probe only after the local surface is stable.
3. Each probe must emit deterministic counts, filtered counts, field lists, and
   one representative row/geometry assertion.
4. Any client-specific branch must be named by PostgreSQL surface, not client
   brand: `pg_attribute`, `pg_type`, cursor suspension, binary params, etc.
5. Unsupported behaviors must be documented in `docs/COMPATIBILITY.md` or this
   file before a broad compatibility claim is made.

## Minimum assertions by profile

| Profile | Required assertions |
|---|---|
| psycopg 3 | startup/auth works; text and binary params round-trip WKB; `ST_AsEWKB` bytes match expected WKB/EWKB; read-only role denies writes; optional COPY path matches Rust COPY oracle |
| SQLAlchemy | engine connects; inspector lists schemas/tables/columns; simple reflected SELECT works; generated SQL does not require unsupported PostgreSQL extension features |
| GeoPandas | `read_postgis` returns expected feature count; geometry column is parsed as WKB; bbox/filter query returns deterministic rows; CRS/SRID behavior is documented |
| pg_featureserv-style | table/layer discovery query works; bbox filter maps to safe spatial predicate; GeoJSON response has expected feature count and properties |
| BI/SQL | projection/filter/grouped aggregate queries run through simple and extended protocol; result types and NULLs match expected rows; query budget metrics are emitted when run against lake profile |
| Martin attributes | representative tile is non-empty; configured attribute tags are present; bbox/tile envelope query remains exact after pruning |

## Data fixtures

Use the smallest deterministic fixture first:

1. stable demo layers from `seed_demo`;
2. LayoutBench `sf0` for pruning/OLAP shapes;
3. Monaco OSM copied layers for real-data client assertions;
4. Overture/GeoParquet-style wide layers only after the earlier fixtures are
   boring.

## Promotion ladder

| Stage | Evidence | Claim allowed |
|---|---|---|
| Trace captured | saved SQL/catalog/protocol trace and failing focused test | no support claim |
| Local/wire gate | Rust or Python probe runs against local QuackGIS | implementation surface is understood |
| Kind client probe | containerized client runs in the maintained Kind network | named client/profile can be listed as opt-in |
| Scheduled/report artifact | metrics/logs uploaded and rendered in compatibility report | maintained compatibility matrix row |
| Real-data matrix | probe runs over copied real-data layers with dashboard | real-data workflow claim for named dataset/client |

## Non-goals unless traces require them

- Full PostgreSQL ORM feature parity.
- PL/pgSQL, triggers, logical replication, advisory locks, or extension install
  semantics.
- Client-specific compatibility hacks without a named PostgreSQL catalog/protocol
  surface.

## Implemented local surface probe

`just api-client-local-smoke` starts a temporary QuackGIS server and runs
`crates/quackgis-server/examples/api_client_probe.rs`. It is deliberately lighter
than named client containers and asserts the protocol/catalog surfaces first:

- psycopg-style text and typed binary WKB parameters plus `ST_AsEWKB` readback;
- SQLAlchemy-style `information_schema` table/column reflection without hidden
  layout columns;
- GeoPandas-style WKB row reads with documented SRID-0 behavior;
- pg_featureserv-style bbox/filter query over a WKB layer;
- BI-style grouped aggregate query; and
- non-empty MVT bytes plus layer/key/value dictionary tokens from
  `ST_AsMVTGeom` + `ST_AsMVT(geom, layer, extent, attrs...)`.

The MVT encoder itself now has a focused unit test for key/value dictionaries and
feature tags. The public SQL/client probes assert representative attribute tags,
and `just martin-e2e` runs the real Martin binary over the synthetic fixture and
requires its configured `name=origin` tag in the returned tile. Copied OSM/Martin
binary runs remain the real-data promotion gate.

This is a local/wire gate, not a named-client support claim. Promotion still
requires the containerized client or real server named in the matrix. The local
gate is now part of `just ci`, so API/client catalog drift is caught before the
heavier Kind/client promotion path.

## Implemented Kind profile probe

`just kind-api-client-probe` runs the same API/profile contract inside the
maintained Kind network using the shared probe ConfigMap. It emits one
`api_client_summary` line for compatibility metrics:

- `feature_count` for GeoPandas-style WKB reads;
- `reflected_columns` for SQLAlchemy-style table/column reflection;
- `bbox_count` for pg_featureserv-style spatial filters;
- `groups` for BI-style grouped aggregates; and
- `tile_bytes` for non-empty MVT output with representative attribute tags.

`just kind-probes`, `just kind-compatibility`, and `just kind-compat-report` now
include this API-client profile row. This promotes the profile to a maintained
scheduled/report artifact, but it still does **not** claim broad named-client
support until the actual psycopg, SQLAlchemy, GeoPandas, pg_featureserv, BI, or
Martin workflows run with their own dependencies/traces.

## Next concrete probes

1. Promote the profile psycopg surface into a real psycopg 3 container probe,
   including read-only write denial and COPY where dependencies are available.
2. Promote the SQLAlchemy/GeoPandas surfaces into named Python dependency probes.
3. Add a pg_featureserv-style server harness once the bbox/filter SQL surface is
   boring.
4. Run Martin MVT attribute tags through the real Martin binary over copied OSM
   layers.
5. Run the named client probes over copied real-data layers before claiming
   real-data workflow support.
