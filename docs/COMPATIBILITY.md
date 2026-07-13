# Compatibility and limitations

QuackGIS currently exposes an owned PostgreSQL wire edge over DuckDB Spatial and
official DuckLake. DataFusion and Sedona SQL execution are no longer part of the
runtime.

The forward target is the bounded PostgreSQL 18 catalog, role, privilege, session,
and role-aware REST profile in
[POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md). Nothing in that plan
is a current compatibility claim unless it is also recorded below and in
[ROADMAP_STATUS.md](./ROADMAP_STATUS.md).

The machine-readable `pg18-column-core-v1` target and digest-pinned PostgreSQL
18.4 result-description oracle are validated by `just project-contract-check`.
They freeze desired behavior and do not imply that QuackGIS currently implements
the listed catalogs or PostgreSQL result types.

The exact OGR 3.11.5 client image also has a credential-free normalized
copied-point trace against digest-pinned PostgreSQL 18.4/PostGIS. Its 21 query
families are an implementation oracle, not evidence that copied spatial discovery
currently succeeds against QuackGIS.

The same oracle now includes exact psql 18.3 `\d+` evidence: 12 normalized catalog
query families and the rendered structure of a five-column spatial table. This is
also a target corpus, not current QuackGIS describe support.

An offscreen, digest-pinned QGIS 3.44.11 PostgreSQL provider oracle also succeeds
for layer open, field/CRS discovery, privilege and owner inquiry, count, 3D extent,
and first-feature binary cursor read. Its 32 statements (26 unique families) are
frozen as targets; the same workflow does not yet pass against QuackGIS.

## Proven local contract

The required real-driver workflow proves:

- PostgreSQL simple and extended query framing through `pgwire`;
- SCRAM-SHA-256 startup and normalized read/write table allowlists;
- required TLS with client-side certificate/hostname verification, plaintext
  denial, and restart-based certificate/password rotation;
- one structurally parsed `SELECT`, `CREATE TABLE`, `INSERT`, `UPDATE`, `DELETE`,
  `BEGIN`, `COMMIT`, or `ROLLBACK` statement; the sole multi-statement exception is
  an all-`SET`, maximum-eight, structurally allowlisted simple-query batch;
- parameterized reads and mutations without SQL interpolation;
- PostgreSQL text `COPY FROM STDIN` for the maintained scalar and WKB types;
- `SET standard_conforming_strings`, encoding, bounded `client_min_messages`, and
  the exact QGIS 3.44 `extra_float_digits`/`application_name`/`datestyle` bootstrap;
  `SHOW search_path`, stable `pg_database` identity, PostgreSQL-shaped
  `current_database`/`current_schema`/`current_schemas`, `public` relation mapping,
  and quoted COPY targets;
- portal paging, transaction isolation, failed-transaction `25P02` enforcement,
  `COMMIT`-as-rollback after failure, disconnect rollback, restart, and reopen;
- the simple-protocol, server-owned
  `CALL quackgis_merge_adjacent_files(...)` maintenance procedure for an
  explicitly configured identity; arbitrary procedures remain unsupported;
- Arrow result encoding for maintained scalar types, with generated WKB payload/
  null and fixed-binary properties; and
- 42 curated spatial cases sent with their original PostGIS spelling through the
  real server, using DuckDB Spatial plus bounded QuackGIS rewrites/macros.

Run the evidence:

```sh
mise run duckdb-bootstrap
mise exec -- just ci
```

## Current client status

| Client/surface | Current status |
|---|---|
| `psql` / PostgreSQL protocol clients | bounded simple/extended protocol supported; TLS/SCRAM scalar smoke passes with psql 18.3 in rootless-Podman Kind |
| `tokio-postgres` | maintained real-driver integration client |
| PostgreSQL text COPY clients | bounded maintained type set supported |
| GDAL/OGR | 3.11.5 TLS/SCRAM scalar smoke passes in Kind after structural encoding/string/search-path probes; copied-data discovery and optional `ST_SRID` remain open |
| QGIS | PostgreSQL 18.4/PostGIS oracle frozen for exact 3.44.11 headless read workflow; QuackGIS execution remains blocked on traced catalog/privilege/spatial surfaces |
| GeoServer, Martin | target; client traces and QuackGIS qualification remain open |
| psycopg | 3.2.13 TLS/SCRAM scalar smoke passes in Kind; copied-data workflow remains open |
| SQLAlchemy, GeoPandas, pg_featureserv | target; named dependency workflows remain open |
| `pg_dump`, logical replication, PL/pgSQL, triggers, LISTEN/NOTIFY | unsupported/non-goals |

## Spatial contract

DuckDB Spatial owns geometry execution. Durable geometry transport is binary
WKB/EWKB through Arrow and pgwire. The server currently rewrites these maintained
function spellings without touching quoted SQL text or comments:

- `ST_MakePoint` → `ST_Point`;
- `ST_NumPoints` → `ST_NPoints`;
- `ST_GeomFromEWKT`, `ST_AsHEXEWKB`, `GeometryType`, `ST_GeometryType`,
  `ST_CurveToLine`, and `ST_HasArc` → bounded QuackGIS-owned DuckDB macros; and
- `postgis_lib_version()` / `postgis_version()` → compatibility markers.

The 57-case ledger currently classifies 31 native DuckDB cases, five rewrites,
six macros, 10 Rust/catalog-edge gaps, and five extension candidates. The first
42 execute through pgwire; the remaining 15 have ledger-pinned `0A000` behavior
through simple and extended protocol. SRID-preserving EWKB behavior, geography,
dimensions, general `ST_GeometryN`, extent/catalog helpers, MVT, and broad PostGIS
catalog surfaces remain open unless a focused test says otherwise.

## Deliberate runtime limits

- Local official DuckLake catalog and local data paths only.
- Maintenance is disabled unless `QUACKGIS_MAINTENANCE_USER` names the caller;
  it remains constrained by the write table allowlist and cannot run inside an
  explicit transaction.
- Global and reader/writer/maintenance admission are bounded. Native gates prove
  the default eight-reader ceiling under 32 clients and simultaneous all-class
  queueing/completion at reduced smoke scale; this is not mixed-workload soak
  evidence.
- The scalar transport profile enforces the 15% pgwire-over-ADBC p50 budget only
  in reference mode and only when direct ADBC lasts at least one second. Reduced
  smoke/local ratios are diagnostic and do not establish the release budget.
- Exact pinned DuckDB library version/digest and preinstalled signed extensions.
- Query results stream one driver-produced Arrow batch at a time and reject a
  driver batch above the configured byte ceiling before pgwire encoding. Clean
  1M/10M generated-BIGINT reference runs stay below the +128 MiB process RSS
  budget with one in-flight batch; a clean 1M nullable VARCHAR/BLOB reference also
  passes across 489 native batches. Maximum driver-batch and additional type
  shapes remain open.
- A fully exhausted result returns its native connection. Closing a suspended
  portal or otherwise dropping a partially delivered result quarantines that
  session. After cancellation, the same client receives a stable internal error
  rather than silently reusing uncertain native state; independent sessions remain
  usable.
- Pgwire cancellation interrupts active DuckDB result and COPY workers. A COPY
  client that sends no further frame does not receive its error until it resumes
  or disconnects. Writes run in a cancellable pre-commit transaction: autocommit
  cancellation rolls back and keeps the session reusable, while cancellation in
  an explicit transaction rolls back and quarantines that session. Commit is
  non-cancellable once entered and failures are indeterminate. A clean
  100-sample long-query reference run passes the 500 ms budget at 1.51 ms p95;
  every cancelled session is quarantined and a fresh session remains usable. When
  a COPY client resumes, deadline cancellation returns `57014` and the target
  remains unchanged even after staging batches were flushed.
- COPY has no total request ceiling, incrementally decodes PostgreSQL text escapes,
  and enforces configured pre-body frontend-frame, post-decode chunk, row, and
  Arrow-batch limits. A header-only oversized declaration closes immediately with
  zero publication; oversized decoded chunks and malformed final rows abort
  staging synchronously. The clean 10M reference passes at 126 MiB RSS delta and
  0.528 of direct ADBC throughput; an idle client receives a timeout/cancel error
  only when it sends another frame or disconnects.
  CSV/binary COPY options, arrays, JSON, time zones, and every scalar type remain
  unsupported.
- Reserved bbox layouts are validated before COPY staging; clients cannot provide
  bbox values, and partial/wrong-type/ambiguous layouts fail with stable `0A000`.
- Direct `INSERT` and reserved bbox UPDATEs on a maintained layout return `0A000`.
  A geometry UPDATE is supported only when its right-hand side is one numbered
  bound parameter (optionally cast) or `NULL`; DuckDB recomputes all four bounds
  in the same statement. Arbitrary geometry expressions and tuple assignments
  remain `0A000`. UPDATEs of ordinary columns preserve existing geometry/bounds,
  and `DELETE` remains supported.
- A one-table exact `ST_Intersects` over the maintained WKB column gains
  conservative four-axis bbox candidates for bounded literal envelopes/text
  geometries and numbered-bound WKB. The exact DuckDB predicate remains in the
  plan. Native exact-oracle comparisons cover holes, boundaries, NULL/empty data,
  empty probes, invalid bow-tie data/probes, bound/literal probes, and reopen;
  pgwire covers the literal path. OR/NOT placement, joins, subqueries, multiple
  matching predicates, and arbitrary or oversized probe expressions are
  deliberately left unoptimized; malformed/ambiguous reserved layouts fail
  closed. This is functional evidence, not a scan-byte or scale claim.
- `pg_catalog`, `information_schema`, broad spatial discovery, and GIS client-specific
  metadata are incomplete. A client-neutral executable fixture structurally maps
  explicit and implicit namespace/database/type/range/collation/owner-role
  references to private views. It proves stable logical database/schema/search-path
  discovery with `name`/`name[]` wire types, 24 exact PostgreSQL 18 profile/QGIS
  built-ins plus
  PostGIS-shaped geometry/geography scalar/array rows, every namespace/owner/
  array/collation link, wire/OID parameter types, and WKB transport. All
  unimplemented/private catalog routing and unsupported wildcard/nested/set/
  derived/implicit-join/CTE/cross-database shapes fail closed. The structurally
  lossy `TABLE` form is rejected globally. This does not claim user-object catalogs,
  RowDescription origins, or named-client execution against QuackGIS. An opt-in
  checksum-pinned DuckLake 1.5.4 development extension now proves the selected
  public column-identity lifecycle contract through ADBC; it is not loaded by
  default, packaged, or release-supported, and does not yet expose user-object
  catalogs.
- Binary columns named `geom_wkb` use the same geometry sentinel OID as the
  maintained COPY bbox layout. RowDescription plus text hex-WKB, binary WKB, and
  NULL transport are tested through pgwire for geometry and the maintained
  `geog` convention for geography; subtype/SRID/dimension catalog identity remains
  open.
- PostgreSQL roles, memberships, object ownership/grants, `SET ROLE`, privilege
  inquiry functions, transaction-local request claims, RLS, and role-aware
  OpenAPI are not implemented. Current startup identities and table allowlists
  must not be described as PostgreSQL RBAC.
- Arrow schema mapping and encoding are tested together for Float16, UInt32 OID
  aliases, Float16/fixed-binary lists, WKB, fixed binary, NULLs, invalid JSON, and
  nested error propagation. Unsupported list layouts fail during schema mapping;
  broader generated temporal/decimal/dictionary/nested coverage remains open.
- Shared PostgreSQL/object-storage DuckLake, multi-writer recovery, migration,
  production packaging, soak, and disaster-recovery evidence remain open.
- Forced drain of an explicit uncommitted transaction has process-level evidence:
  same-path restart preserves the committed row, exposes none of the uncommitted
  row, and accepts a new write. General write/commit interruption, relocated
  recovery, and release-catalog recovery timing remain open.

See [DUCKDB_SPATIAL_GAP_LEDGER.md](./DUCKDB_SPATIAL_GAP_LEDGER.md) and
[ENGINE_CAPABILITY_LEDGER.md](./ENGINE_CAPABILITY_LEDGER.md) for detailed gaps.
