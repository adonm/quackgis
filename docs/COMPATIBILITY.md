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

The shared `quackgis-edge` crate implements the cryptographic I0 protocol and an
executable local-direct seam: bounded bootstrap-signed one-worker leases,
registered credential-key proofs bound to the current iroh endpoint, fresh worker
challenges, typed streams, mandatory uncompressed plus optional adaptive LZ4
negotiation, fail-closed relay selection, a loopback or mutual-TLS tiny client, and a worker
bridge that binds pgwire startup to the leased role without carrying SCRAM. `just iroh-protocol-test` proves the
pure contract; `just iroh-direct-smoke` uses real local iroh endpoints and a fake
trust-mode pgwire backend. `just iroh-duckdb-smoke` additionally proves typed and
spatial queries plus differential direct-TCP/iroh result/type/error, parameter,
portal, transaction/disconnect, successful and malformed COPY atomicity,
cancellation/quarantine, concurrent-session, and fresh-reconnect outcomes against
the current DuckDB/DuckLake worker. The same differential oracle passes a forced
custom relay and the opt-in public relay preset. Clean 8/32/64 MiB transport
profiles publish and enforce direct/relay CPU, RSS, latency, throughput,
cancellation, stream, byte, codec, and decode budgets. K0 packages the direct
path with one mTLS bridge and loopback-only worker; the resource measurements
remain host evidence, not packaged WAN or hosted-relay SLOs.

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
- optional immutable role provisioning with exact LOGIN/auth matching, stable
  explicit OIDs, acyclic PostgreSQL 18 membership-edge options, and bounded owner/
  grant declarations; actual pgwire proves `session_user`/`current_user`/
  `current_role`/`user`, `SET [SESSION|LOCAL] ROLE`, `NONE`, reset, assumption
  denial, connection isolation, prepared invalidation, and local cleanup after
  commit/rollback/failed-transaction rollback;
- common configured authorization for schema USAGE plus table ownership and
  SELECT/INSERT/UPDATE/DELETE/MAINTAIN; ownership, direct grants, inherited
  `inherit_option=true` grants, and `PUBLIC` feed one decision before DuckDB,
  while legacy allowlists remain a non-widening outer ceiling;
- relational `pg_roles` and `pg_auth_members` projections for the immutable
  graph, including stable explicit role/edge OIDs, resolving role/member/
  bootstrap-grantor references, LOGIN/INHERIT and membership options, fixed
  non-superuser fields, and NULL credential material;
- bounded PostgreSQL 18 `pg_has_role`, `has_schema_privilege`,
  `has_table_privilege`, `has_any_column_privilege`, and
  `has_column_privilege` inquiry from the same role decisions as execution,
  including comma-separated privilege literals, ownership/direct/inherited/
  PUBLIC grants, and false grant/admin options; name-literal objects work in the
  official lane, while OID/catalog-expression and exact column lookup require
  durable catalog identity;
- role-bound PostgreSQL 18 `information_schema.schemata`, `tables`, `columns`,
  `table_privileges`, `role_table_grants`, `column_privileges`, and
  `role_column_grants`; authoritative DuckDB metadata supplies existing objects
  and columns, effective-role decisions are intersected with the legacy
  identity/allowlist ceiling, identifier/character fields advertise
  `name`/`varchar`, PUBLIC is excluded from `role_*`, and `MAINTAIN` is excluded
  from the standard privilege views;
- in the checksum-pinned identity lane, DuckLake-derived defaults and table/column
  comments through `pg_attrdef`, `pg_description`, `pg_get_expr`,
  `col_description`, and `obj_description`; result descriptions preserve
  PostgreSQL `oid`/`int2`/`pg_node_tree`/`text` identity, metadata participates in
  the schema fingerprint epoch, and effective-role visibility is intersected
  with the login identity's legacy allowlist ceiling;
- durable PostgreSQL 18 `pg_constraint` rows for DuckLake's native named
  `NOT NULL` constraints, including rename-stable OIDs, resolving namespace/
  relation/attribute references, `int2[]` column keys, and
  `pg_get_constraintdef`; `pg_index` has its maintained PostgreSQL shape and
  OID-22 `int2vector` wire identity but is empty, because DuckLake cannot enforce
  primary, unique, foreign-key, check, or index semantics;
- role-bound generic `geometry_columns` rows for native geometry and maintained
  WKB geometry-name sentinels, typed-empty `spatial_ref_sys`, discoverability
  through `information_schema.tables`, empty-geometry `ST_SRID`, DuckDB-backed
  version probes, and textual `ST_Extent`/`ST_3DExtent`; the contract reports
  `GEOMETRY`, dimension 2, SRID 0 rather than inferring unenforced metadata;
- exact transaction-local `request.jwt.claims` assignment through one text literal
  or `$1`, bounded at 16 KiB/setting and 32 KiB/session, plus PostgreSQL `text`
  retrieval with NULL-on-missing behavior; actual pgwire proves outside-
  transaction and oversized denial, stale-prepare rejection, and commit/
  failed-transaction rollback cleanup; cancellation inside a role/context
  transaction proves failed-state denial, native quarantine, and empty fresh-
  session identity/context;
- portal paging, transaction isolation, failed-transaction `25P02` enforcement,
  `COMMIT`-as-rollback after failure, disconnect rollback, restart, and reopen;
- the simple-protocol, server-owned
  `CALL quackgis_merge_adjacent_files(...)` maintenance procedure for an
  explicitly configured identity; arbitrary procedures remain unsupported;
- Arrow result encoding for maintained scalar types, with generated WKB payload/
  null and fixed-binary properties; and
- 43 curated spatial cases sent with their original PostGIS spelling through the
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
| GDAL/OGR | 3.11.5 TLS/SCRAM scalar smoke passes in Kind; traced generic spatial metadata/SRID/extent query shapes pass focused actual pgwire, while the copied-data client workflow and no-FID behavior remain open |
| QGIS | PostgreSQL 18.4/PostGIS oracle frozen for exact 3.44.11 headless read workflow; focused traced spatial metadata/version/extent surfaces pass, but full QuackGIS client execution and generic SRID 0 behavior remain unqualified |
| GeoServer, Martin | target; client traces and QuackGIS qualification remain open |
| psycopg | 3.2.13 TLS/SCRAM scalar smoke passes in Kind; copied-data workflow remains open |
| SQLAlchemy, GeoPandas, pg_featureserv | target; named dependency workflows remain open |
| `pg_dump`, logical replication, PL/pgSQL, triggers, LISTEN/NOTIFY | unsupported/non-goals |
| Tiny iroh client bridge | executable direct, forced-custom-relay, and opt-in public-default-relay seams differentially match direct TCP for maintained result/type/error, simple/extended parameter/portal, spatial, transaction/disconnect, COPY atomicity, cancellation/quarantine, concurrent-session, and reconnect behavior. The packaged direct Kind path passes pinned psql/psycopg/OGR, direct/plaintext/certificate-free denial, ordered reconnect, and mTLS/edge-key rotation with old-certificate denial. Adaptive LZ4 host budgets pass; packaged resource and hosted-relay qualification remain open |

## Spatial contract

DuckDB Spatial owns geometry execution. Durable geometry transport is binary
WKB/EWKB through Arrow and pgwire. The server currently rewrites these maintained
function spellings without touching quoted SQL text or comments:

- `ST_MakePoint` → `ST_Point`;
- `ST_NumPoints` → `ST_NPoints`;
- `ST_GeomFromEWKT`, `ST_AsHEXEWKB`, `GeometryType`, `ST_GeometryType`,
  `ST_CurveToLine`, `ST_HasArc`, `ST_SRID`, `ST_Extent`, and `ST_3DExtent` →
  bounded QuackGIS-owned DuckDB macros; and
- `postgis_lib_version()`, `postgis_version()`, `postgis_geos_version()`, and
  `postgis_proj_version()` → compatibility/runtime markers.

The 57-case ledger currently classifies 31 native DuckDB cases, five rewrites,
seven macros, nine Rust/catalog-edge gaps, and five extension candidates. The first
43 execute through pgwire; the remaining 14 have ledger-pinned `0A000` behavior
through simple and extended protocol. SRID assignment/preservation, geography,
authoritative subtype/dimension/CRS metadata, PostGIS box wire identity, general
`ST_GeometryN`, `Find_SRID`, MVT, and broad PostGIS catalog surfaces remain open
unless a focused test says otherwise.

## Deliberate runtime limits

- Local official DuckLake catalog and local data paths only.
- The I0 tiny client and worker bridge package the K0 local direct ingress but are
  not yet a published/hosted-relay release. The backend remains loopback-only and
  trust-mode, immediately returns
  `AuthenticationOk`, and the exact leased startup role; nested TLS/GSS and
  password/SASL challenges are rejected. Direct, custom-relay, and opt-in public-
  relay native gates prove the maintained differential DuckDB oracle. The
  K0 exposes only a mutual-TLS tiny-client Service and denies direct worker TCP,
  plaintext, missing-certificate, and rotated-old-certificate access. The
  transport profile isolates framing/codec cost with an echo backend, so its host
  budgets do not claim packaged resource, hosted-relay, or WAN behavior.
- Maintenance is disabled unless `QUACKGIS_MAINTENANCE_USER` names the caller;
  it remains constrained by the write table allowlist and cannot run inside an
  explicit transaction.
- Configured owners and grants now enforce maintained table operations and feed
  bounded privilege inquiry plus role-aware schema/table/column/grant discovery.
  Legacy read/write/maintenance identities and table allowlists remain an
  outer ceiling. Role switching cannot inherit the login's coarse access unless
  the effective role has the matching configured grant. `SET LOCAL ROLE` outside a transaction fails
  with `25001`, a bounded divergence from PostgreSQL's warning/no-op behavior.
- Arbitrary `set_config`/`current_setting` names, non-local assignment, embedded
  setter query shapes, NUL, and DuckDB-qualified setting functions fail before
  native execution. Request context is not a row policy and has no authorization
  effect before the independent RLS milestone.
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
  closed. Clean registered 100k, 1M, and consecutive 10M point profiles compare
  25 ordered official-DuckLake files for this layout and native `GEOMETRY`.
  Exact pgwire counts agree, each plan keeps its exact `ST_Intersects` recheck,
  and conservative compressed scan-byte upper bounds stay below 5%. Both 10M
  layouts compact from 25 files to one without result changes. This does not yet
  qualify non-point shapes, broad analytical workloads, or 100M scale.
- `pg_catalog`, unmaintained `information_schema`, broad spatial discovery, and
  GIS client-specific metadata are incomplete. A client-neutral executable
  fixture structurally maps explicit and implicit namespace/database/type/range/collation/owner-role
  references to private views. It proves stable logical database/schema/search-path
  discovery with `name`/`name[]` wire types, 24 exact PostgreSQL 18 profile/QGIS
  built-ins plus
  PostGIS-shaped geometry/geography scalar/array rows, every namespace/owner/
  array/collation link, wire/OID parameter types, and WKB transport. All
  unimplemented/private catalog routing and unsupported wildcard/nested/set/
  derived/implicit-join/CTE/cross-database shapes fail closed. The structurally
  lossy `TABLE` form is rejected globally. This does not claim named-client
  execution against QuackGIS. An opt-in
  checksum-pinned DuckLake 1.5.4 development extension now proves the selected
  public column-identity lifecycle contract through ADBC. That lane also proves
  durable namespace/relation/row-type OID and per-table attribute-number
  allocation plus commit/rollback/reopen/drop-recreate schema epochs in protected
  DuckLake control tables. Concurrent server sessions serialize commit plus
  reconciliation; all public development create paths pass the gate, and direct
  or dynamically indirect pgwire access to registry tables is denied. Empty
  schemas remain unsupported because the selected API emits no durable identity
  for them. The development lane now exposes current base tables through guarded,
  registry-backed `pg_namespace`, `pg_class`, `pg_attribute`, and composite
  `pg_type` rows. Actual pgwire joins prove stable rename/reopen identity,
  attribute tombstones, drop/recreate, non-public schemas, scalar/spatial type
  references, direct-column/wildcard RowDescription origins, unsupported-type
  rejection, and prepared-read epoch invalidation. It also proves PostgreSQL-
  typed strict/nullable `regclass`/`regtype`/`regnamespace`/`regrole`, maintained
  search-path/quoted-name resolution, OID/text casts, aliases/arrays/typmods,
  `format_type`, and exact foundation descriptions from the client-neutral
  profile. The same actual-pgwire lane now proves role/legacy-filtered default and
  comment rows/functions. DuckDB's implicit string `NULL` default marker is
  normalized to no catalog row; explicit `DEFAULT NULL` is not separately
  distinguished, and pgwire comment/default DDL remains unsupported. This closes
  the first shared traced structural slice. It also exposes stable NOT-NULL
  constraint identity and a truthfully empty index catalog; QuackGIS does not
  infer primary/unique keys from data or non-null columns. Baseline startup still
  rejects those user-object catalogs explicitly. This lane is not loaded by default, packaged,
  or release-supported.
- Binary columns named `geom_wkb` use the same geometry sentinel OID as the
  maintained COPY bbox layout. RowDescription plus text hex-WKB, binary WKB, and
  NULL transport are tested through pgwire for geometry and the maintained
  `geog` convention for geography; subtype/SRID/dimension catalog identity remains
  open.
- Mutable PostgreSQL role/grant DDL, RLS, and role-aware OpenAPI are not
  implemented. The immutable configured role model is table/operation RBAC, not
  RLS; legacy startup identities and table allowlists remain a separate outer
  ceiling.
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
