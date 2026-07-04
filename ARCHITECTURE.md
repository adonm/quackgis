# Architecture

QuackGIS is a PostGIS-compatible spatial database server in a single Rust
binary. It speaks the PostgreSQL wire protocol but does **not** run PostgreSQL.
Three DataFusion-native components share one `SessionContext`:

| Component | Upstream | Role |
|---|---|---|
| [datafusion-postgres](https://github.com/datafusion-contrib/datafusion-postgres) | datafusion-contrib | pgwire server, auth/RBAC, TLS, extended query protocol; `datafusion-pg-catalog` (pg_catalog + information_schema), `arrow-pg` (Arrow↔PG type/OID mapping) |
| [Apache SedonaDB](https://github.com/apache/sedona-db) | Apache Sedona | Spatial execution: ST_* kernels (GeoRust/GEOS/PROJ), geometry/geography types, CRS propagation, spatial joins, GeoParquet |
| [datafusion-ducklake](https://github.com/datafusion-contrib/datafusion-ducklake) | datafusion-contrib | DuckLake lakehouse: catalog metadata in SQL DB, data in Parquet on file/object storage |

QuackGIS itself is the thin integration layer: PostGIS SQL surface, client
compatibility shims, and spatial table layout. Upstreams are consumed as
**pinned forks** (`[patch.crates-io]` / git revs) so missing capabilities are
built immediately in-fork — see the gap ledger in
[ROADMAP.md](./ROADMAP.md).

## Layer model

```text
┌──────────────────────────────────────────────────────────────┐
│ PostgreSQL clients                                           │
│ QGIS · GeoServer (JDBC) · psql · psycopg · GDAL/OGR · BI     │
├──────────────────────────────────────────────────────────────┤
│ datafusion-postgres                                          │
│ pgwire · simple+extended protocol · auth · TLS · portals     │
├──────────────────────────────────────────────────────────────┤
│ quackgis compatibility layer (owned code)                    │
│ geometry type over the wire (OID + WKB/EWKB text/binary)     │
│ geometry_columns · spatial_ref_sys · postgis_version()       │
│ pg_catalog additions · session no-ops (SET, client GUCs)     │
│ cursor + introspection shims for QGIS/GeoServer              │
├──────────────────────────────────────────────────────────────┤
│ SedonaDB session (DataFusion SessionContext)                 │
│ ST_* functions · geometry/geography · CRS · spatial joins    │
├──────────────────────────────────────────────────────────────┤
│ datafusion-ducklake                                          │
│ DuckLake CatalogProvider · Parquet scan/write · snapshots    │
├──────────────────────────────────────────────────────────────┤
│ Storage                                                      │
│ catalog DB (SQLite / PostgreSQL†) · Parquet on file/S3       │
└──────────────────────────────────────────────────────────────┘
† catalog-metadata-only; a few MB of SQL rows, not a data engine.
```

One process, one binary. No PostgreSQL server, no DuckDB, no C extensions, no
extension ABI coupling.

## Design principles

1. **Wire compatibility, not Postgres.** Running full PostgreSQL to get pgwire
   was the v0.1 approach; it cost a PG server, a vendored pg_ducklake fork, a C
   extension for the geometry type, and a DuckDB-extension ABI treadmill. The
   target clients (QGIS, GeoServer, OGR, psycopg) need protocol + catalog +
   PostGIS SQL surface — all servable from Rust.

2. **Pinned upstreams, fork-preferred for gaps.** The best design needs
   capabilities that do not all exist upstream yet (DuckLake UPDATE/DELETE and
   pruning, SQL cursors, deep pg_catalog, SedonaDB wire encodings — see the
   gap ledger in ROADMAP.md). All upstreams are Apache-2.0, so we pin exact
   revisions and fork/vendor the moment a capability is missing, shipping from
   the fork. Each fork logs its divergence (`DIVERGENCE.md`) and rebases at
   milestone boundaries; upstreaming happens opportunistically from the fork,
   never on the critical path.

3. **SedonaDB is the spatial engine.** No reimplemented kernels. QuackGIS
   registers SedonaDB's function catalog and adds only PostGIS-compat aliases
   and signature adapters where names/arities differ.

4. **DuckLake is the only table storage.** Tables live as Parquet + DuckLake
   catalog metadata. Interoperable with DuckDB's `ducklake` extension and
   anything else that reads the spec.

5. **Client-driven compatibility.** The definition of done is scripted QGIS and
   GeoServer workflows passing against the server, not a function-count.

## Geometry over the wire

PostGIS clients exchange geometry as (hex-)EWKB with a server-assigned type
OID. SedonaDB represents geometry as WKB-encoded Arrow arrays with CRS
metadata. The compatibility layer:

- registers a `geometry` (and `geography`) type OID in the emulated
  `pg_type`, stable across sessions;
- encodes result columns as hex-EWKB (text protocol) / EWKB (binary protocol)
  via an `arrow-pg` extension point;
- decodes bound parameters from WKB/EWKB/WKT into SedonaDB geometry;
- carries SRID via EWKB flags backed by DuckLake column metadata.

This mirrors the approach datafusion-postgres already ships behind its
`postgis` feature flag (backed by geodatafusion); QuackGIS swaps the function
catalog for SedonaDB's larger one.

## Catalog and introspection

- `datafusion-pg-catalog` provides `pg_catalog` (pg_class, pg_namespace,
  pg_attribute, pg_type, pg_database, …) and `information_schema` views over
  the DataFusion catalog — DuckLake tables appear automatically.
- QuackGIS adds the PostGIS metadata surface: `geometry_columns`,
  `geography_columns`, `spatial_ref_sys` (from PROJ/EPSG data),
  `postgis_version()`, `postgis_lib_version()`, `postgis_full_version()`.
- QGIS/GeoServer introspection queries (pg_index for keys, regclass casts,
  `version()`, format_type) are test fixtures; gaps are fixed in our
  datafusion-pg-catalog fork where general (gap ledger G2), here where
  PostGIS-specific.

## DuckLake spatial layout

Unchanged from v0.1 — spatial tables materialize deterministic layout columns:

| Column | Purpose |
|---|---|
| `minx/miny/maxx/maxy` | File-level zone-map pruning |
| `spatial_cell` (quadkey) | Partition pruning |
| `spatial_sort` (Hilbert) | Spatial clustering within files |

Query: cell prune → bbox prune → exact predicate. Stages 1–2 are performance;
stage 3 is correctness. Pruning happens in the DataFusion scan against DuckLake
file statistics — missing upstream, built in our datafusion-ducklake fork
(gap ledger G7).

## Trust boundaries

1. **Client connections**: datafusion-postgres owns auth (password/RBAC) and
   TLS; startup fails closed without credentials configured.
2. **Client SQL**: parsed by DataFusion's sqlparser; PostGIS-dialect rewrites
   are explicit, deny-by-default (no string-level regex rewriting).
3. **Geometry**: WKB/EWKB validated at the wire boundary before entering
   SedonaDB; invalid input is a client error, never a panic.
4. **Storage**: DuckLake catalog DB owns metadata transactions; object-store
   credentials are deployment secrets, never stored in the catalog.

## What changed from v0.1

| v0.1 (retired) | Replaced by |
|---|---|
| PostgreSQL 18 server + auth + catalog | datafusion-postgres + datafusion-pg-catalog |
| Vendored pg_ducklake fork (table AM, query routing) | datafusion-ducklake |
| DuckDB + `sedonadb` DuckDB extension (this repo's `src/`) | Apache SedonaDB crates, used natively |
| `pg_geometry` C extension (type, typmods, casts) | Rust type mapping in arrow-pg extension point |
| ~160 PG-level SQL function stubs (`container/init.d/`) | Functions execute in-engine; no stubs |
| ~500 MB container (PG + DuckDB + GDAL + extensions) | Single Rust binary, target < 100 MB image |

## Non-goals

- Running PostgreSQL or DuckDB in any form.
- PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication.
- Full PostgreSQL SQL surface — target is what spatial clients actually send.
- Topology schema, Tiger geocoder, SFCGAL.
- GiST indexes (DuckLake layout columns + scan pruning instead).
