# Architecture

QuackGIS is a PostGIS-compatible SQL and control plane for a Sedona-powered
spatial lakehouse. A single Rust service exposes pgwire, plans and executes with
DataFusion + SedonaDB, and persists tables through DuckLake catalog metadata plus
Parquet files. PostgreSQL may hold DuckLake metadata in the scaled profile, but it
is never the query engine or user-table store.

This document owns architecture, current implementation boundaries, and product
invariants. Forward outcomes belong in [ROADMAP.md](./ROADMAP.md); implemented
evidence belongs in [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) and the
linked compatibility/operations documents.

## System identity

The primary user is a platform or application team operating a shared spatial
lake. QuackGIS should support many stateless readers, parallel ingest/edit jobs,
selective spatial requests, broad columnar analytics, recoverable maintenance,
and familiar GIS/API clients without copying the data into PostgreSQL or DuckDB.

Three DataFusion-native dependencies provide the core engine:

| Component | Role | QuackGIS posture |
|---|---|---|
| `datafusion-postgres` + `datafusion-pg-catalog` + `arrow-pg` | pgwire, parser/planner integration, PostgreSQL catalogs, parameter/result encoding, authentication, TLS | vendored fork because QuackGIS needs parser-boundary, catalog, cursor, and spatial type behavior |
| Apache SedonaDB crates | exact spatial functions, geometry execution, CRS/projection support | pinned DF54-aligned fork; no QuackGIS kernel reimplementation |
| `datafusion-ducklake` | DuckLake catalogs, Parquet scans/writes, snapshots, metadata writers | vendored fork for atomic mutation and positional-row semantics |

QuackGIS owns the integration policy: PostGIS aliases, compatibility surfaces,
spatial layout, snapshot/time-travel routing, mutation/maintenance orchestration,
authorization, metrics, and release evidence. Every forked behavior is recorded in
[DIVERGENCE.md](./DIVERGENCE.md) and
[docs/DUCKLAKE_ALIGNMENT.md](./docs/DUCKLAKE_ALIGNMENT.md).

## Layer model

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Clients                                                              │
│ QGIS · GeoServer · Martin · GDAL/OGR · psql · drivers · APIs · BI    │
├──────────────────────────────────────────────────────────────────────┤
│ pgwire boundary                                                      │
│ startup/auth/TLS · simple/extended parse · COPY state · portals      │
│ raw SQL compatibility rewrites · Arrow↔PG types/OIDs/row encoding    │
├──────────────────────────────────────────────────────────────────────┤
│ QuackGIS policy                                                      │
│ PostGIS/catalog surfaces · authorization · query-shape validation    │
│ snapshot routing · spatial pruning · DML/compaction · metrics         │
├──────────────────────────────────────────────────────────────────────┤
│ DataFusion + SedonaDB                                                │
│ logical/physical planning · exact ST_* execution · OLAP expressions  │
├──────────────────────────────────────────────────────────────────────┤
│ datafusion-ducklake                                                  │
│ catalog providers · Parquet scan/write · snapshots · metadata commit │
├──────────────────────────────────────────────────────────────────────┤
│ Storage profiles                                                     │
│ SQLite catalog + local files │ PostgreSQL metadata + object storage  │
└──────────────────────────────────────────────────────────────────────┘
```

The QuackGIS binary has no PostgreSQL server, DuckDB, C-extension ABI, or native
GEOS/GDAL runtime requirement. Native client/test containers may still contain
those tools.

## Architectural invariants

1. **PostGIS compatibility is an edge contract.** It does not move PostgreSQL into
   the data plane.
2. **SedonaDB decides exact spatial truth.** Hidden layout predicates are candidate
   filters only.
3. **DuckLake snapshot publication is the write boundary.** A mutation becomes
   visible once or not at all.
4. **WKB/EWKB is the stable geometry interchange.** Richer in-memory/type metadata
   may evolve without changing durable bytes or maintained client encodings.
5. **Protocol, catalog, and data authorization are separate boundaries.** Metadata
   that says a user can edit is not itself authorization.
6. **Storage divergence is named.** A working QuackGIS backend is not automatically
   a standard DuckLake backend.
7. **Trace fixtures precede compatibility code.** New behavior is classified by a
   reusable SQL/protocol surface.
8. **Heavy assets are indexed, not decoded, in the SQL hot path.**
9. **Every large claim has a smaller deterministic oracle.**

## Storage profiles and interoperability truth

| Profile | Current role | Compatibility statement |
|---|---|---|
| SQLite catalog + local Parquet | deterministic preview, tests, local persistence, backup/restore oracle | spec-oriented single-catalog path, but not yet a drop-in DuckDB-writable catalog because allocator columns differ |
| PostgreSQL metadata + S3-compatible objects | multi-pod Alpha, parallel readers/writers, managed-service target | currently uses `datafusion-ducklake`'s library-specific multicatalog schema; it is not interchangeable with a standard DuckLake PostgreSQL catalog unless a reference-reader gate proves otherwise |

Both profiles should expose the same QuackGIS SQL semantics, but they do not yet
carry the same external interoperability claim. The PostgreSQL backend must either
converge on standard DuckLake behavior or gain an explicit export/migration path
before 1.0. QuackGIS APIs must not expose private catalog table layouts.

Object writes and catalog commits are intentionally separate phases. A process may
prewrite a Parquet/delete object and fail before the metadata commit, leaving an
orphan candidate. It may never expose only half of a SQL mutation. Orphan
inventory, quarantine, and cleanup are operations concerns, not reasons to weaken
the catalog transaction.

## Pgwire and SQL compatibility boundaries

Experience with QGIS, OGR, GeoServer, Martin, pgjdbc, and parser-level time travel
showed that “a query hook” is not one boundary. Compatibility work belongs in four
ordered layers:

1. **Raw SQL preprocessing before parsing.** Narrow syntax the PostgreSQL dialect
   cannot parse is tokenized and lowered here. `AS OF SNAPSHOT <id>` is rewritten
   to the existing named snapshot selector. Rewrites must ignore quoted/commented
   text, recognize an unambiguous grammar fragment, and leave unsupported forms to
   fail in the parser.
2. **Parsed statement hooks.** DuckLake DDL/DML, transaction statements, safe
   spatial pruning, snapshot query-shape checks, and catalog compatibility use the
   AST. Authorization is applied near mutation planning.
3. **Catalog and wire encoding.** `pg_catalog`, `geometry_columns`, PostgreSQL type
   OIDs, parameter deserialization, Arrow result schemas, and text/binary row
   encoding are observable client contracts distinct from SQL execution.
4. **Stateful protocol handlers.** `COPY FROM STDIN`, cursor state, and portal
   behavior cannot be modeled as ordinary statement rewrites. COPY has a dedicated
   pgwire subprotocol handler. Simple/libpq cursor paths work for maintained traces;
   general extended-protocol portal suspension/`Execute.max_rows` remains a known
   boundary until a real client requires and verifies it.

Hook order is deliberate: QuackGIS DuckLake policy, catalog compatibility, cursor,
SET/SHOW, and transaction hooks share one session service. A new hook must state
which boundary owns its validation and whether it changes SQL, planning, result
shape, or connection state.

## Catalog compatibility

`datafusion-pg-catalog` supplies the broad PostgreSQL catalog skeleton. QuackGIS
adds PostGIS metadata/functions and focused shape compatibility for `pg_class`,
`pg_attribute`, `pg_index`, `pg_type`, OGR, pgjdbc, and cursor surfaces.

The maintainable rule is surface-oriented compatibility:

- preserve the exact row labels, OIDs, parameter types, nullability, and binary/
  text formats clients observe;
- reduce captured client SQL to the smallest reusable classifier and test;
- prefer real catalog derivation over synthetic shortcuts;
- use synthetic responses only when the underlying PostgreSQL concept does not
  exist in QuackGIS and the result is safe; and
- never treat privilege helper output as an authorization decision.

String/shape classifiers remain migration debt. As the maintained client matrix
grows, common classifiers should become parser/AST rules or real catalog data
rather than an expanding collection of query-text signatures.

## Geometry representation and identity

### Current representation

| Layer | Current representation |
|---|---|
| Durable data | WKB/EWKB bytes in Parquet Binary columns; the current Rust DuckLake writer records Binary as `blob` |
| Execution | Arrow Binary/BinaryView WKB arrays consumed by SedonaDB, plus ordinary hidden layout columns |
| Wire text | hex WKB/EWKB encoded under QuackGIS's advertised spatial type |
| Wire binary | raw WKB/EWKB bytes using bytea-compatible serializers |
| PostgreSQL identity | sentinel `geometry`/`geography` OIDs 90001/90002, resolvable through QuackGIS catalog shims |
| Discovery | conventional geometry column names (`geom`, `geometry`, `footprint`, OGR/QGIS variants) plus QuackGIS metadata surfaces |

The current path is intentionally WKB-first and works for maintained clients, but
two details must not be overstated:

1. sentinel OIDs are stable QuackGIS identifiers, not dynamic OIDs from a real
   PostGIS installation; and
2. current durable DuckLake column metadata does not yet carry enough geometry
   identity to eliminate naming heuristics.

### Target identity

QuackGIS should record spec-aligned `GEOMETRY`/geography metadata (or a stable
upstream UDT/type mechanism) alongside WKB without changing the bytes. That
metadata should drive `geometry_columns`, SRID/dimension discovery, reference
reader behavior, and schema evolution. Until then, accepted naming conventions
and sentinel-OID behavior are part of the explicit compatibility contract.

CRS authority/code, WKT2/PROJJSON, vertical datum, coordinate epoch, transform
pipeline, accuracy, and conversion tolerance belong in durable table/column or
sidecar metadata when workflows need reproducible high-accuracy transforms. They
must not be inferred from one row or silently discarded during layout projection.

## Spatial-temporal layout and pruning

Spatial writes currently materialize ordinary hidden columns:

- `_qg_minx`, `_qg_miny`, `_qg_maxx`, `_qg_maxy` from one WKB bounds pass;
- `_qg_space_bucket` for coarse spatial grouping;
- `_qg_time_bucket` from a recognized numeric time column where available; and
- `_qg_space_sort` as a Morton/Z-order locality key.

These are **ordinary Parquet columns**, not current DuckLake partition columns.
The present skip path is mostly Parquet row-group/file statistics plus injected
hidden predicates. It must not be described as a partition index until the writer
and reader actually publish and prune DuckLake partitions.

The pruning rule is deny-by-default:

1. recognize a bounded single-table predicate and safe optional time shape;
2. prove the target has required hidden columns;
3. reject joins, unsafe top-level `OR`, wildcard/projection hazards, and text found
   only in strings/comments;
4. add bbox/time candidate predicates; and
5. retain the exact original SedonaDB predicate.

COPY and transaction grouping are layout primitives because they let QuackGIS
sort a useful batch before writing. Fragmented autocommit files are repaired by
compaction. Current bucket compaction uses positional delete + pending append in
one mutation when lineage planning succeeds; the full-table replacement remains a
correct fallback. See
[docs/DUCKLAKE_SPATIAL_LAYOUT.md](./docs/DUCKLAKE_SPATIAL_LAYOUT.md).

The target layout may add true coarse DuckLake partitions, adaptive time/bucket
policies, richer stats, Bloom filters, or an upstream spatial layout primitive.
Those are migrations only after exact-result, catalog-growth, writer-fanout, and
reference-reader gates pass.

## Query and analytical execution

QuackGIS uses DataFusion for vectorized projection, filtering, expressions,
aggregates, joins, and physical execution; SedonaDB supplies exact spatial
kernels. The desired analytical shape is:

1. prune candidate files/row groups with ordinary attributes and safe hidden
   layout predicates;
2. keep projections narrow and push primitive expressions/aggregates near the
   Parquet scan where supported;
3. fan out grouped/window/join analysis only with measured plans and budgets; and
4. apply exact SedonaDB predicates before returning spatially filtered rows.

Success is measured with rows, files, row groups, bytes, candidate counts,
catalog roundtrips, latency percentiles, and plan evidence—not query success alone.

## Mutation and transaction semantics

### Autocommit native mutation

Supported DELETE/UPDATE and bucket-compaction shapes use a fork-backed
`TableMutation` boundary:

1. pin the relevant snapshot and live data/delete generations;
2. resolve physical `(file, row_position)` lineage without repartitioning or
   filter/sort transformations that could change positions;
3. prewrite cumulative positional delete files and, for UPDATE/compaction, a
   pending replacement data file;
4. compare expected live generations; and
5. publish delete generations, appends, and retirements in one catalog snapshot.

Physical row identity is a correctness mode, not an ordinary optimized scan.
Repartitioning, pruning, or pushdown that changes row positions is disabled for
those scans. Performance work must preserve that isolation rather than assuming
normal query optimizations are safe.

`RETURNING` rows are collected before publication; an error before the metadata
commit leaves no visible mutation. Process death after object prewrite may leave
orphans. Stale generation conflicts are errors callers can retry, not a reason to
silently replan against a new head.

### Explicit transactions

The current explicit transaction contract is intentionally narrower than
PostgreSQL:

- first DML touch pins one DuckLake table generation;
- statements rewrite a private connection-local staged table;
- COMMIT writes/publishes the final table under one replacement snapshot;
- explicit ROLLBACK and a failed COMMIT discard staged state; an ordinary
  statement error is not yet full PostgreSQL aborted-transaction emulation;
- concurrent replacement conflicts fail closed; and
- the maintained `ALTER TABLE ... ADD COLUMN` edit workflow can be staged, but
  other DDL, multi-table atomic writes, and read-your-writes SELECT overlays are
  not generally provided.

Native batching should replace the full-table staging path only when it preserves
the same one-visible-snapshot contract. See
[docs/NATIVE_DML_FORK_PLAN.md](./docs/NATIVE_DML_FORK_PLAN.md).

## Snapshot reads and dataset history

Literal `AS OF SNAPSHOT <id>` is lowered before PostgreSQL parsing to a named
snapshot selector. Parsing is only the first safety step: QuackGIS then accepts one
simple snapshot-qualified table, rejects joins/multiple selectors/non-literal ids,
checks that the schema/table existed at the requested snapshot, creates a
query-scoped snapshot-pinned DuckLake catalog, and discards it after the query.
The normal catalog remains at the current head.

Timestamp resolution, protected retention, rollback, branch/release workflows,
and CDC are separate product semantics. CDC row functions stay disabled until
projection shape, ordering, bounds, update/delete meaning, and simple/extended
wire behavior are deterministic. See
[docs/SNAPSHOT_OPERATIONS.md](./docs/SNAPSHOT_OPERATIONS.md).

## Shared catalogs and service scaling

QuackGIS processes are intended to be replaceable. Durable coordination belongs
in the DuckLake catalog/object store; connection-local portal/COPY/transaction
state belongs in one process. Shared readers refresh catalog state according to an
explicit interval, while mutation and maintenance paths require a strong refresh
before planning.

Refresh is both correctness and cost policy: low latency improves edit visibility;
relaxed refresh can reduce catalog pressure for stable analytical readers. Both
modes need metrics and managed-service budgets. No in-process cache may become an
unrecorded source of durable truth.

## Security and observability boundaries

1. **Client transport and identity.** Trust mode is the local-development default.
   Password mode uses SCRAM and fails startup when required credentials are absent;
   TLS fails startup when only half configured. Production deployment must enforce
   TLS or a trusted mTLS/proxy boundary.
2. **SQL authorization.** Coarse read-only/read-write policy is enforced in the
   DuckLake SQL hook near recognized DDL/DML/COPY/maintenance planning. Future
   table/schema rules must cover data, catalogs, metadata UDTFs, snapshots, and
   maintenance consistently.
3. **Catalog/object credentials.** These are infrastructure identities scoped to
   the catalog database/schema and object prefix. They are independent of pgwire
   users and rotate independently.
4. **Metrics and logs.** Process counters and structured events omit SQL text,
   usernames where unnecessary, object paths, and secrets. Metrics are not an
   authorization surface and must bind to a private/trusted endpoint.
5. **Compatibility metadata.** `pg_roles` and privilege helpers help clients infer
   editability; they never grant access by themselves.

See [docs/SECURITY_RBAC.md](./docs/SECURITY_RBAC.md).

## Multi-modal asset model

Large assets enter QuackGIS through a stable SQL identity and queryable footprint
or derived geometry, with source content in object storage. A useful asset record
needs more than a URI:

- stable collection/asset/source-object identity and version;
- footprint/3D bounds and acquisition time;
- media/format, size, resolution/point spacing/quality;
- CRS, vertical datum, coordinate epoch, and transform provenance;
- lineage, checksum/etag, lifecycle state, and retention relationship to dataset
  releases; and
- authorization semantics for revealing and dereferencing the URI.

Applications query, aggregate, and narrow candidates through SQL, then fetch or
stream the heavy object with a format-specific service. QuackGIS should add a
decoder only when a measured query requires one and the dependency does not
destabilize the simple-feature runtime. See
[docs/MULTIMODAL_ASSETS.md](./docs/MULTIMODAL_ASSETS.md).

## Lessons that shape future work

- Correct over-selection is preferable to clever false-negative pruning.
- Bulk ingest and transaction grouping often improve layout more than row-level
  write micro-optimizations.
- A protocol-compatible schema is not enough; field count, labels, OIDs, formats,
  parameter typing, and connection state all matter.
- Parser compatibility does not make a feature safe; query-shape validation and
  query-scoped state are still required.
- Safe native mutation is defined by one catalog publication, not by how quickly
  objects were written.
- Physical row lineage requires a deliberately less-optimized scan path.
- Local mesh evidence is valuable but cannot substitute for managed-service
  rotation, provider failure, backup, or restore evidence.
- Metadata inspection is useful before row-level CDC; an introspection feature
  must never panic or misproject.
- Name-based geometry discovery and SQL-text classifiers were pragmatic Alpha
  tools, but durable metadata and structural parsing are the long-term shape.
- Ambitious scale claims need exact rows/bytes/profile/cost and must distinguish
  routine, scheduled, and manual stress evidence.

## Non-goals

- Running PostgreSQL as the query engine or user-table store.
- Embedding DuckDB.
- Acting as an OLTP application database or document database.
- Full PostgreSQL SQL/extension compatibility.
- PL/pgSQL, triggers, LISTEN/NOTIFY, or logical replication.
- Topology schema, Tiger geocoder, or SFCGAL.
- Requiring mutable GiST/R-tree side indexes for correctness.
- Decoding every raster, point-cloud, CAD/BIM, or mesh format in the pgwire
  process.
