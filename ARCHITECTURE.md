# Architecture

QuackGIS is a PostGIS-compatible front door to a Sedona-powered spatial lakehouse
in a single Rust binary. It is built first for platform/application developers
who need to answer large, complex spatial questions over shared DuckLake/Parquet
data with high throughput, horizontal read scaling, and columnar OLAP analysis.
The long-term architecture also treats raster, point-cloud, CAD/BIM, 3D tile, and
reality-capture datasets as queryable asset indexes plus sidecars rather than as
heavy decoders in the SQL hot path.

QuackGIS speaks the PostgreSQL wire protocol but does **not** run PostgreSQL as a
query engine. Three DataFusion-native components share one `SessionContext`:

| Component | Upstream | Role |
|---|---|---|
| [datafusion-postgres](https://github.com/datafusion-contrib/datafusion-postgres) | datafusion-contrib | pgwire server, auth/RBAC, TLS, extended query protocol; `datafusion-pg-catalog` (pg_catalog + information_schema), `arrow-pg` (Arrow↔PG type/OID mapping) |
| [Apache SedonaDB](https://github.com/apache/sedona-db) | Apache Sedona | Spatial execution: ST_* kernels via Rust-native features in the QuackGIS build (`geo`, `tg`, `proj-rust`), geometry/geography types, CRS propagation, spatial joins, GeoParquet |
| [datafusion-ducklake](https://github.com/datafusion-contrib/datafusion-ducklake) | datafusion-contrib | Rust-native DuckLake lakehouse targeting the official DuckLake 1.0+ spec: catalog metadata in SQL DB, data in Parquet on file/object storage |

QuackGIS itself is the thin integration layer: PostGIS SQL surface, client
compatibility shims, and spatial table layout. Upstreams are consumed as
**tracked fork branches** so missing capabilities are
built immediately in-fork — see the gap ledger in
[ROADMAP.md](./ROADMAP.md).

## Layer model

```text
┌──────────────────────────────────────────────────────────────┐
│ PostgreSQL clients                                           │
│ QGIS · GeoServer · Martin · GDAL/OGR · psql · psycopg · Python │
├──────────────────────────────────────────────────────────────┤
│ datafusion-postgres                                          │
│ pgwire · simple+extended protocol · SCRAM auth · TLS · portals │
├──────────────────────────────────────────────────────────────┤
│ quackgis compatibility layer (owned code)                    │
│ geometry type over the wire (OID + WKB/EWKB text/binary)     │
│ geometry_columns · spatial_ref_sys · postgis_version()       │
│ pg_catalog additions · session no-ops (SET, client GUCs)     │
│ pg_catalog/cursor shims for PostGIS-style clients            │
├──────────────────────────────────────────────────────────────┤
│ SedonaDB session (DataFusion SessionContext)                 │
│ ST_* functions · geometry/geography · CRS · spatial joins    │
│ vectorized columnar projections · aggregates · expressions   │
├──────────────────────────────────────────────────────────────┤
│ datafusion-ducklake                                          │
│ DuckLake CatalogProvider · Parquet scan/write · snapshots    │
├──────────────────────────────────────────────────────────────┤
│ DuckLake storage profiles                                    │
│ SQLite catalog + local Parquet files                         │
│ PostgreSQL catalog + S3 Parquet objects                      │
└──────────────────────────────────────────────────────────────┘
PostgreSQL, when used, is catalog metadata only; it is not the query engine or user table storage.
```

One QuackGIS process, one binary. No PostgreSQL server in the query/data plane,
no DuckDB, no C extensions, no extension ABI coupling, and no native
GEOS/PROJ/GDAL runtime dependency for the QuackGIS binary. A PostgreSQL database
may be used as DuckLake catalog metadata storage in the scaled profile.

## Design principles

1. **Scaled spatial questions first.** The primary job is high-performance
   spatial SQL over big lakehouse datasets: many stateless QuackGIS readers,
   many parallel ingest jobs, one shared DuckLake catalog/object prefix, and
   SedonaDB exact spatial execution. Compatibility work exists to keep existing
   PostGIS clients and tools usable against that lakehouse.

2. **DuckDB-style OLAP ergonomics, without DuckDB.** Users should be able to run
   fanout analytical SQL over column-oriented DuckLake/Parquet data: scan many
   geometries, compute grouped spatial/attribute stats, use primitive aggregates
   and calculations, push filters/projections down where possible, then recheck
   exact SedonaDB predicates for the narrowed result. DataFusion/Sedona/DuckLake
   provide this path; DuckDB is not embedded.

3. **Wire compatibility, not Postgres.** Running full PostgreSQL to get pgwire
   was the v0.1 approach; it cost a PG server, a vendored pg_ducklake fork, a C
   extension for the geometry type, and a DuckDB-extension ABI treadmill. The
   target clients (QGIS, GeoServer, Martin, GDAL/OGR/`ogr2ogr`, psycopg) need protocol + catalog +
   PostGIS SQL surface — all servable from Rust.

4. **Pinned upstreams, fork-preferred for gaps.** The best design needs
   capabilities that do not all exist upstream yet (DuckLake UPDATE/DELETE and
   pruning, SQL cursors, deep pg_catalog, SedonaDB wire encodings — see the
   gap ledger in ROADMAP.md). All upstreams are Apache-2.0, so we track upstream heads through fork branches and fork/vendor the moment a capability is missing, shipping from
   the fork branch. Each fork logs its divergence (`DIVERGENCE.md`) and rebases at
   milestone boundaries; upstreaming happens opportunistically from the fork,
   never on the critical path.

5. **SedonaDB is the spatial engine.** No reimplemented kernels. QuackGIS
   registers SedonaDB's function catalog and adds only PostGIS-compat aliases
   and signature adapters where names/arities differ.

6. **DuckLake is the only table storage and is a core product path.** Tables live
   as Parquet + DuckLake catalog metadata. SQLite + local files and PostgreSQL +
   S3 are both first-class storage profiles; the PostgreSQL/S3 profile is the
   scaled multi-writer/high-QPS deployment target. Extending datafusion-ducklake
   for QuackGIS requirements is in scope, but changes must remain
   forward-compatible with the official DuckLake 1.0+ spec and interoperable with
   reference DuckLake readers where possible. When upstream DuckLake stabilizes
   equivalent primitives — deletion-vector/Puffin updates, protected snapshots,
   branch/merge, materialized views, VARIANT/UDT/fixed-size-array support, Bloom
   filters, metadata-scan improvements, or PostgreSQL catalog roundtrip reductions
   — QuackGIS should migrate toward them instead of preserving private storage
   semantics.

7. **Client-driven compatibility.** The definition of done is scripted QGIS,
   GeoServer, Martin, Python/SQL, and OGR/`ogr2ogr` workflows passing against the
   server, not a function-count. The compatibility promise is “common PostGIS GIS
   clients and tools work without significant changes,” not “QuackGIS is
   PostgreSQL.”

8. **Operations and metadata are product features.** DuckLake metadata UDTFs,
   `pg_roles`, privilege helpers, compatibility reports, trendable metrics,
   backup/restore oracles, and catalog refresh behavior are part of the public
   platform contract. A lakehouse that cannot be inspected or restored is not
   credible no matter how fast a single query is.

9. **Heavy spatial formats enter through index rows and sidecars first.** Raster,
   point-cloud, CAD/BIM, 3D tiles, and reality-capture content should expose
   footprints, CRS/epoch metadata, quality/resolution fields, lineage, and object
   URIs in SQL while preserving high-fidelity artifacts outside the vectorized
   query path until a specific reader/kernel is justified.

## Lessons from the preview/Alpha gates

1. **Exact recheck is the safety rail.** Hidden bbox/layout predicates are only a
   prefilter. Every public spatial rewrite must preserve the original SedonaDB
   predicate, and every new predicate shape needs an exact-vs-pruned test before
   it is enabled. The scanner now ignores comments/strings and skips unsafe
   top-level `OR` predicates because false pruning is worse than a slower scan.
2. **Compatibility shims must be organized by catalog surface.** Early one-off
   client branches worked, but the maintainable shape is `pg_class`,
   `pg_attribute`, `pg_index`, `pg_type`, pgjdbc, OGR metadata, and cursor
   surfaces with trace-shaped unit tests. New client gaps should first be reduced
   to one of those surfaces.
3. **COPY and transaction grouping are layout features.** They are not just ingest
   conveniences. Bulk/grouped writes produce larger, better-ordered Parquet units
   and much better pruning than fragmented autocommit inserts; compaction is the
   repair path when clients cannot batch writes.
4. **Object-store reads need explicit budgets.** The useful regression signal is
   not only query success; QPS/OLAP probes now assert bytes scanned and file-group
   ceilings from `EXPLAIN ANALYZE` and emit trendable metrics.
5. **Cheap gates protect expensive gates.** Unit tests, static probe validation,
   and report rendering checks catch drift before a Kind image build or external
   storage run. Every new large/manual probe should have a small deterministic
   companion gate.
6. **Auth and write authorization have different trust boundaries.** SCRAM/TLS
   authenticate the pgwire session; DuckLake SQL hooks still enforce write
   authorization close to mutation planning. Catalog privilege metadata helps
   clients choose editability but is not the write boundary.
7. **Native DML is only safe when it publishes once.** Positional delete files,
   staged replacement data files, and bucket compaction metadata must become
   visible under one DuckLake snapshot. Prewritten objects can be cleanup work;
   partially visible mutations are correctness bugs. Upstream multi-deletion-vector
   Puffin support should be adopted only with reference-reader interop and the same
   single-snapshot visibility guarantee.

## Geometry strategy: EWKB everywhere with a real type OID

The goal is the highest performance/fidelity tradeoff SedonaDB can support today.
EWKB is the current PostGIS wire standard. GeoArrow is useful metadata and a
future native-array optimization, but it is not the primary physical format for
the current layout path. We use EWKB/WKB at every durable/client boundary:

```text
┌────────────┐     WKB Binary     ┌────────────┐    EWKB bytes    ┌────────────┐
│  Parquet   │ ◄─────────────────►│  SedonaDB  │ ◄──────────────► │   pgwire   │
│  (storage) │  column_type =     │ (WKB Arrow │  geometry OID    │ (clients)  │
│            │  "GEOMETRY"        │  Binary)   │  text=hex-EWKB   │            │
└────────────┘                    └────────────┘  binary=EWKB     └────────────┘
```

| Layer | Representation | Rationale |
|---|---|---|
| **Storage** | WKB in Parquet Binary columns; DuckLake `column_type = "GEOMETRY"`; optional Arrow `geoarrow.wkb` metadata | Forward-compatible with DuckLake 1.0+ and GeoParquet; compact columnar; `geometry_columns` view can discover geometry columns from catalog metadata without scanning data |
| **Execution** | WKB in Arrow Binary arrays (SedonaDB 0.4 default) plus hidden layout columns | No format pivot; SedonaDB's Rust-native kernels operate on WKB, and QuackGIS computes bbox/layout once per write batch |
| **Wire (text)** | hex-EWKB string behind a real `geometry` type OID | What `psql` and text-protocol clients display; identical to PostGIS |
| **Wire (binary)** | raw EWKB bytes behind the same OID | What QGIS/Martin/GeoServer binary cursors and prepared-statement binary params expect; 2× bandwidth saving vs hex-text |
| **SRID/epoch/fidelity metadata** | SRID in EWKB plus DuckLake/table metadata for CRS WKT2/projjson, vertical datum, coordinate epoch, transform pipeline, accuracy, and conversion tolerance | End-to-end CRS propagation and reproducible high-accuracy CAD/aerial transforms without a separate `geometry_columns` lookup per-row |

### DuckLake geometry column tagging

DuckLake stores column types as strings in `ducklake_column.column_type`. The
spec recognises `GEOMETRY` as a valid type; datafusion-ducklake maps it to
Arrow `Binary` internally. QuackGIS marks geometry columns with
`column_type = "GEOMETRY"` so that:

- the `geometry_columns` view can be populated from catalog metadata alone (no
  data scan needed to discover which columns hold spatial data);
- DuckDB's reference `ducklake` extension interoperates (it also recognises
  GEOMETRY columns);
- the DuckLake 1.0+ spec is respected (GEOMETRY is a spec-defined type string).

### Implementation path (G1 + G13)

1. **G1 (arrow-pg fork):** register a `geometry` type OID in `pg_type` with
   text encoding = hex-EWKB, binary encoding = raw EWKB. Encode SedonaDB
   Binary/WKB result columns as EWKB (prepend SRID flag from column metadata).
   Decode inbound parameters from EWKB/WKB/WKT.
2. **G13 (Martin/PostGIS surface):** `PostGIS_Lib_Version()` constant UDF;
   `geometry_columns` view from DuckLake catalog metadata;
   `spatial_ref_sys` table from PROJ/EPSG; verify SedonaDB covers
   `ST_AsMVT`, `ST_AsMVTGeom`, `ST_TileEnvelope`, `ST_Transform`,
   `ST_Expand`, `ST_CurveToLine`, `&&` operator.

This mirrors the approach datafusion-postgres already ships behind its
`postgis` feature flag (backed by geodatafusion); QuackGIS swaps the function
catalog for SedonaDB's larger one.

GeoArrow direction: tag WKB Arrow fields as `geoarrow.wkb` when doing so is
interoperable, but keep durable geometry bytes as WKB/EWKB. A local GeoArrow 0.8
probe confirmed that Arrow Binary + `geoarrow.wkb` metadata is recognized and
row-readable; it did not provide a better batch bbox primitive than Sedona's WKB
bounds path, so M5 layout is WKB-first.

CAD/reality-capture direction: queryable SQL columns stay OGC simple-feature
`geometry`/`geography` WKB/EWKB. High-fidelity source content such as CAD curves,
splines, meshes, point clouds, rasters, and BIM objects is represented as derived
query geometries plus provenance/asset sidecars rather than destructively forcing
everything into simple polygons. See the spatial layout design for the type tiers
and coordinate-epoch metadata.

## Catalog and introspection

- `datafusion-pg-catalog` provides `pg_catalog` (pg_class, pg_namespace,
  pg_attribute, pg_type, pg_database, …) and `information_schema` views over
  the DataFusion catalog — DuckLake tables appear automatically.
- QuackGIS adds the PostGIS metadata surface: `geometry_columns`,
  `geography_columns`, `spatial_ref_sys` (from PROJ/EPSG data),
  `postgis_version()`, `postgis_lib_version()`, `postgis_full_version()`.
- Client introspection queries (pg_index for keys, regclass casts,
  `format_type`, pg_type/pg_class/pg_attribute shape) are test fixtures; gaps
  are fixed in our datafusion-pg-catalog fork where general (gap ledger G2), and
  in QuackGIS's `CatalogCompatHook` where PostGIS/wire-boundary specific.
- The hook is intentionally surface-oriented. Trace SQL should become focused
  tests for the PostgreSQL surface it exercises before adding another client-name
  special case.

## DuckLake spatial layout

Spatial tables materialize deterministic hidden layout columns at write time:
bbox, coarse spatial bucket, and spatial sort key today; temporal bounds and time
buckets are the next layout extension. QuackGIS computes geometry-derived layout
values in one WKB pass. Query planning prunes with safe hidden bbox predicates
above DuckLake/Parquet statistics, then always rechecks the exact SedonaDB
predicate.

The design deliberately avoids mutable GiST/R-tree side indexes. Parallel writers
can write independent files and publish DuckLake snapshots. Whole-table
compaction is explicit and commits through one replacement snapshot; bucket-
targeted compaction uses native positional deletes plus one pending replacement
file when row-lineage planning succeeds, so small-file repair can stay scoped to
one coarse time/space bucket.

Layout is part of write strategy as much as read strategy: COPY and explicit
transaction grouping let QuackGIS sort larger batches by hidden layout keys,
whereas many tiny autocommit inserts create fragmented files that must later be
compacted. Query gates therefore track file groups, row groups, and bytes scanned,
not just result counts.

See [DuckLake spatial-temporal layout](docs/DUCKLAKE_SPATIAL_LAYOUT.md) for the
automatic partitioning/indexing direction, including huge aerial captures,
local-coordinate CAD data, geography/geometry type tiers, coordinate drift
metadata, and trillion-row table targets (gap ledger G7).

## Columnar OLAP analysis

QuackGIS should feel familiar to users who reach for DuckDB to ask ad hoc
analytical questions over columnar data, but it keeps the QuackGIS stack:
DataFusion planning/execution, SedonaDB spatial kernels, and DuckLake/Parquet
storage.

Target query shape:

1. fan out over a large spatial table or asset index;
2. use hidden layout columns and ordinary Parquet statistics to prune early;
3. compute grouped spatial/attribute statistics with vectorized projections,
   primitive aggregates, conditional expressions, and joins where supported;
4. use those calculated values as filters for relevant rows/assets;
5. reapply exact SedonaDB spatial predicates before returning results or serving a
   PostGIS client.

This complements, rather than replaces, PostGIS compatibility. QGIS/GDAL/GeoServer
need a familiar pgwire/PostGIS surface; platform services also need large fanout
analytics such as coverage summaries, asset inventory stats, quality-control
metrics, and candidate narrowing before expensive exact spatial operations.

## Transaction semantics over DuckLake snapshots

Autocommit QuackGIS DML remains correct-but-coarse: each
`INSERT`/`UPDATE`/`DELETE` is written through the DuckLake writer API and
published as its own snapshot.

Explicit transaction blocks now provide a DuckLake-native, transactionish path
for edit-client DML on one table:

1. **Pin on first touch.** The first DML statement for a table opens a public
   `datafusion-ducklake` table write session in `Replace` mode. That captures the
   DuckLake base snapshot used for optimistic conflict detection.
2. **Stage writes, do not publish.** QuackGIS materializes the table into a
   private in-memory table for the connection. Later `INSERT`/`UPDATE`/`DELETE`
   statements in the same transaction read and rewrite that staged table; the
   visible DuckLake snapshot is unchanged.
3. **Publish at `COMMIT`.** The staged final table is written through the held
   DuckLake writer session and published as one DuckLake snapshot. If another
   writer published a newer generation after the base snapshot, DuckLake conflict
   detection aborts the commit and the client must retry.
4. **Discard at `ROLLBACK` or error.** QuackGIS drops the private staged table;
   the visible DuckLake snapshot never changes.

This is intentionally narrower than PostgreSQL transaction emulation: DDL inside
explicit transactions and multi-table write transactions fail closed, and
ordinary `SELECT` statements inside a transaction still read the committed
DuckLake catalog rather than the private staged table. Native delete files,
partial-file rewrites, and multi-table single-snapshot commits remain future
performance/semantic hardening; the current semantic boundary is a single-table
snapshot commit.

## Trust boundaries

1. **Client connections**: datafusion-postgres owns auth (password/RBAC) and
   TLS; startup fails closed without credentials configured.
2. **Client SQL**: parsed by DataFusion's sqlparser where possible; narrow
   PostGIS/client rewrites use scanner-aware string handling, are explicit,
   deny-by-default, and must preserve exact predicates unless they are pure
   catalog/protocol shims.
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

- Running PostgreSQL as the query engine or user table store. PostgreSQL may be
  used as DuckLake catalog metadata storage.
- Running DuckDB in-process.
- A document database.
- An OLTP application database.
- PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication.
- Full PostgreSQL SQL surface — target is what spatial clients actually send.
- Topology schema, Tiger geocoder, SFCGAL.
- GiST indexes (DuckLake layout columns + scan pruning instead).
