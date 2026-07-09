# API and SQL client probe contract

QGIS, GDAL/OGR, GeoServer, Martin, `psql`, and tokio-postgres/psycopg-style wire
paths are the maintained compatibility base. The next client expansion should be
trace-driven and cheap before it becomes another heavy Kind workflow.

This document defines the support bar for Python/API/BI-style clients.

## Candidate clients

| Client/profile | Target workflow | Support status |
|---|---|---|
| psycopg 3 | connect, parameterized WKB/EWKB, binary/text reads, COPY where applicable | covered indirectly by pgwire/tokio-postgres tests; dedicated probe pending |
| SQLAlchemy Core | reflection enough for read queries and inserts into declared schemas | not claimed |
| GeoPandas `read_postgis` | read WKB-backed geometry into a GeoDataFrame, preserve CRS metadata when available | not claimed |
| pg_featureserv-like reader | table discovery, bbox filters, JSON/GeoJSON-shaped API reads over pgwire | not claimed |
| BI/SQL tools | simple/extended protocol SELECTs, projection/filter/grouped aggregates | not claimed as a named client |
| MVT consumers through Martin | table discovery, non-empty tile, attribute tags | Martin SQL/E2E base exists; attribute matrix pending |

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

## Next concrete probes

1. psycopg 3 local probe for binary WKB/EWKB params and read-only write denial.
2. SQLAlchemy inspector probe over `public.demo_points` and a DuckLake table.
3. GeoPandas `read_postgis` probe over a WKB-backed demo layer.
4. Martin MVT attribute-tag assertion over copied OSM or demo layers.
5. pg_featureserv-style bbox/filter query harness before adding the real server.
