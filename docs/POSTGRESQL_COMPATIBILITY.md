# PostgreSQL catalog, role, and REST compatibility plan

This document defines the target PostgreSQL-facing contract and the practical work
needed to deliver it. It complements:

- [PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md), which owns product goals and
  boundaries;
- [../ARCHITECTURE.md](../ARCHITECTURE.md), which owns implementation boundaries;
- [../ROADMAP.md](../ROADMAP.md), which owns milestone ordering and exit gates; and
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md), which records only implemented evidence.

## Product outcome

QuackGIS will provide one PostgreSQL-compatible identity, authorization, catalog,
and SQL-session boundary over DuckDB Spatial and official DuckLake. PostgreSQL and
GIS clients, the PostgREST-style HTTP edge, and future services must observe the
same answers for:

- authenticated, session, and effective role;
- role membership and role assumption;
- object ownership and operation privileges;
- visible schemas, relations, columns, types, keys, and relationships;
- PostgreSQL wire type and object identity;
- transaction-local request context; and
- actual query and mutation authorization.

For Local 1.0, `quackgis-rest` remains a stateless pgwire client of the packaged
tiny client rather than a direct worker client. It must not own an independent
schema cache authority, role model, or row-policy implementation. Its exposure
configuration is an additional HTTP ceiling, never a replacement for database
privileges. Shared 1.x carries HTTP through the same tiny iroh edge connection to
the assigned complete worker, while retaining this one catalog, identity, and
authorization contract.

## Scope boundary

The target is a versioned, tested PostgreSQL compatibility profile, not a claim
that DuckDB is PostgreSQL internally.

The initial profile targets PostgreSQL 18 behavior needed by:

- psql 18;
- current psycopg 3;
- GDAL/OGR 3.11;
- QGIS 3.44;
- the maintained `pg-rest-server` query/schema core; and
- QuackGIS's role-aware read/OpenAPI HTTP surface.

Numeric OIDs for dynamic objects need not equal those from another PostgreSQL
installation. Catalog relations, OID references, result types, visibility,
privilege decisions, SQLSTATEs, and transaction/session behavior must be
self-consistent. PostgreSQL physical internals such as heap files, TOAST, WAL,
VACUUM state, replication, and MVCC tuple identifiers are not emulated unless a
maintained client requires a truthful mapping.

Full PostgreSQL SQL, `pg_dump` fidelity, PL/pgSQL, triggers, logical replication,
and complete PostgREST compatibility remain non-goals.

## Current floor

The current runtime has a bounded compatibility contract:

- when an immutable role file is configured, DuckDB-derived role-bound maintained
  `information_schema` views expose visible schemas, tables, columns, table
  grants, and table-derived column grants;
- `public` relation names are structurally mapped to DuckDB/DuckLake `main`;
- geometry and geography use two maintained pgwire type OIDs;
- process-local relational `pg_namespace`, `pg_type`, and `pg_range` views expose
  the two spatial OIDs through structurally rewritten explicit catalog references;
- RowDescription and text/binary/NULL WKB transport are tested;
- immutable roles, memberships, grants, role switching, and privilege inquiry
  use one authorization decision; and
- broad unmaintained `pg_catalog`/`information_schema` access fails closed.

The supported source/artifact-pinned identity lane additionally provides stable
user-object relation/attribute identity and RowDescription origins. Authoritative
spatial metadata and key/index semantics remain incomplete. Role-aware
REST/OpenAPI passes directly and in two packaged replicas. The pinned lane
provides shared monotonic epochs; signed-only startup uses exact role-filtered
revision fallback. The pinned lane covers defaults/comments and DuckLake's only
supported constraint (`NOT NULL`), while publishing an empty index catalog rather
than inventing primary/unique identity; this is not a broad PostgreSQL catalog
claim.

The first target contract is frozen in
`tests/fixtures/postgresql18_compatibility_profile.json`. Its normalized result
types come from PostgreSQL 18.4 image digest
`sha256:0c49c0c906cb405ea65e70c284570fee91c7750ca9336369afc0edf4fce211db`
and are recorded in
`tests/fixtures/postgresql18_column_core_reference.json`. The profile covers the
current custom-type resolver, core relation/column identity, PostgreSQL-facing
REST table/column discovery, and maintained session probes. The exact OGR 3.11.5
copied-point discovery trace is frozen separately in
`tests/fixtures/ogr_3_11_5_postgresql18_trace.json`; exact psql 18.3 `\d+` SQL and
rendered table structure are frozen in
`tests/fixtures/psql_18_3_postgresql18_describe_trace.json`; and 32 statements from
an exact offscreen QGIS 3.44.11 open/inspect/count/extent/read workflow are frozen
in `tests/fixtures/qgis_3_44_postgresql18_trace.json`. Role/OpenAPI traces remain
pending on C4/C5; no behavior is inferred from another version.

## Required invariants

### One source of schema truth

- DuckDB and official DuckLake remain authoritative for user schemas, tables,
  columns, constraints, and data.
- QuackGIS may cache and project that metadata, but must not independently decide
  that a user table or column exists.
- Local compatibility identity and catalog epochs may use protected metadata
  written through the supported DuckDB/DuckLake transaction path. Shared users,
  SCRAM verifiers, client credentials, roles, memberships, grants, policy, worker
  pools and assignments, revocation, and security/configuration epochs live in a
  protected transactional PostgreSQL control database. Both stores are versioned,
  backed up with their profile, and separate from official DuckLake metadata.
- A compatibility OID mapping records identity, not a duplicate table definition.

### One authorization decision

- The effective role used by `current_user`, privilege inquiry functions,
  OpenAPI generation, and statement authorization is the same value.
- `has_table_privilege`, `has_column_privilege`, and `pg_has_role` must call the
  same authorization implementation used before native prepare/execution.
- Compatibility rewrites may change execution spelling but never authorization
  targets.
- Unknown object, statement, role, privilege, or policy shapes fail closed.

### PostgreSQL visibility semantics

Catalog visibility and execution permission are distinct:

- maintained `pg_catalog` relations follow PostgreSQL's relation-specific
  visibility rather than globally hiding every unauthorized object;
- maintained `information_schema` views apply PostgreSQL-compatible ownership and
  privilege filters;
- `pg_roles` masks credentials;
- `pg_authid` and QuackGIS internal control metadata are restricted; and
- the REST exposure list can hide HTTP resources without changing direct pgwire
  catalog semantics.

If an operator later requires object names themselves to be confidential, that
must be an explicit restrictive mode and documented as a PostgreSQL divergence.

### Stable identity

- Standard PostgreSQL built-in types retain their standard OIDs for the selected
  compatibility profile.
- QuackGIS compatibility types such as geometry and geography have reserved,
  documented OIDs.
- Dynamic schema, relation, role, and routine OIDs remain stable across normal
  restart and every rename operation QuackGIS supports within one logical catalog
  lifetime.
- Every nonzero reference in the maintained published columns resolves: namespace,
  owner, relation, attribute, type, constraint, routine, and role references
  cannot dangle. Unsupported reference-bearing columns are omitted from the
  profile or use a documented PostgreSQL-valid not-applicable value.
- RowDescription source relation OID and attribute number agree with `pg_class`
  and `pg_attribute` for direct table columns.
- Transient DuckDB object OIDs are not exposed as PostgreSQL identities: observed
  DuckDB relation OIDs change on rename and reopen.

A feasibility slice must first determine whether durable DuckLake object IDs can
anchor this mapping. If they cannot, a minimal transactional compatibility OID
registry is required. Name hashes alone are insufficient because PostgreSQL OIDs
survive rename.

`just duckdb-catalog-identity-test` closes that feasibility question for the
pinned runtime. DuckLake `table_id`/`table_uuid` survive table rename and an
independent process reopen; Parquet `field_id` proves the same column ID before
and after column rename; drop/recreate receives a new table identity. These are
durable compatibility-registry keys, not PostgreSQL OIDs: they are not nonzero
uint32 values with PostgreSQL lifecycle semantics. A small transactional mapping
for namespace/relation OIDs and durable attribute numbers is therefore required.

The lower-maintenance extraction boundary is now decided: obtain an upstream
public DuckLake function rather than bind QuackGIS to private metadata tables or
carry a version-pinned specification adapter. The required
`ducklake_column_info(catalog)` contract returns exactly `schema_name`,
`schema_id`, `schema_uuid`, `table_name`, `table_id`, `table_uuid`, `column_name`,
and `column_id` for each current top-level base-table column in the transaction's
pinned committed DuckLake snapshot. Views, nested child fields, and uncommitted
DDL are excluded; committed empty/new columns are included. Numeric IDs remain
DuckLake `BIGINT` identities, and a column key is scoped by its table identity.

The tracked patch against DuckLake commit
`84ef2d14a0161f6f6197d6c8d2b4dbc45bf40375` passes focused lifecycle,
transaction, nesting, view, and exact-schema tests plus the complete upstream
function-test group. Its accepted artifact is pinned to the DuckDB 1.5.4 ABI and
passes QuackGIS's exact-schema, transaction, rename, reopen, add, and
drop/recreate contract through ADBC. Exact source/patch/artifact pins and the
unsigned native-code boundary are recorded in `PINNED_DUCKLAKE.md`.

The explicit paired path/digest policy is supported for Local 1.0 and packaged in
the runtime image. Private attachment names do not leak into client SQL; upstream
acceptance remains the preferred deletion path rather than a release blocker.

The pinned lane implements the first mapping slice in protected DuckLake tables
under `_quackgis`. `main` maps to namespace OID 2200; other
namespace, relation, and reserved relation-row-type OIDs allocate from 100000
above maintained reservations;
attribute numbers allocate monotonically per durable table UUID. Mappings survive
rename/restart and remain as tombstones after drop so recreation cannot reuse
identity. Because the public function exposes committed state by design,
reconciliation is a separate atomic transaction after the user commit but before
success is returned. A process-wide lock serializes that commit pair across all
sessions, and startup reconciliation covers a process-crash gap. A failed
post-commit reconciliation is reported as a committed change, quarantines the
session, and fatally closes an explicit pgwire transaction. All public
write/create APIs use the same hook. Uniqueness/reference/coverage
invariants are explicitly validated because DuckLake 1.5 has no primary-key or
unique constraints. Direct pgwire relation references to the private schema,
dynamic query indirection, and direct identity-function calls are denied.

`ducklake_column_info` has no row for an empty schema, so this slice deliberately
does not assign one a name-based OID or advance the identity epoch. Pgwire
`CREATE SCHEMA` remains unsupported. Durable empty-schema identity needs an
upstream schema-level API before that surface can be claimed.

### Transaction and cache correctness

- Each catalog snapshot has a monotonic schema/security epoch.
- Committed DDL or authorization changes advance the applicable epoch.
- Rolled-back changes never become visible.
- Prepared statements and REST schema caches cannot continue using an incompatible
  stale snapshot.
- `SET LOCAL ROLE` and transaction-local request settings disappear on commit,
  rollback, failed-transaction cleanup, cancellation, and disconnect.
- A pooled or reused native connection never inherits another pgwire session's
  role or request context.

The pinned registry maintains both halves of the first invariant. A
committed identity fingerprint advances the durable schema epoch for create,
rename, add, drop, recreate, default, and comment changes, while rollback and
restart without change do not. A canonical, credential-free projection of the
immutable role/grant catalogs and outer table ceilings advances the security
epoch only when authorization semantics change. The pair is exposed as
zero-argument `BIGINT` functions and unsupported runtimes fail with `0A000`.
Extended-protocol reads pin the schema epoch at parse time and reject execution
after a change; direct-column origins are resolved from the same guarded registry
snapshot. REST keys caches by role, both epochs, and connection generation,
brackets refreshes with equal epoch reads, and uses exact role-filtered revisions
when the capability is absent. The pinned runtime artifact makes this the packaged cache path; the complete
Kind cache/client rerun remains open.

Read-only PostgreSQL SQL cursors now share that prepared-statement and epoch
boundary. Simple and extended protocol accept plain transaction control plus one
parameter-free `DECLARE ... CURSOR FOR SELECT`, metadata-only `FETCH 0`, and
bounded forward `FETCH`/`CLOSE`. The simple protocol additionally accepts only
the frozen QGIS two-statement shapes `BEGIN READ ONLY; DECLARE ... BINARY CURSOR`
and `CLOSE ...; COMMIT|ROLLBACK`. The transaction mode is enforced at both DML/DDL
and COPY ingest entry points with SQLSTATE `25006`; it is not syntax-only. A
binary declaration forces every result column to PostgreSQL binary format,
including metadata-only and EOF fetches. `ST_AsBinary(..., 'NDR')` maps maintained
WKB/BLOB and native geometry to raw WKB through a bounded NDR-only macro. A session may retain at most 16 cursors and one
fetch may request at most 4,096 rows. The native stream starts on first non-zero
fetch, pins its requested result format, and is drained in bounded pages before
close or transaction end so an unfinished ADBC reader is not reused.
Scroll/hold declarations, non-NDR `ST_AsBinary`, backward/absolute movement, and
format changes on a live non-binary cursor fail closed.

Catalog subqueries remain fail-closed except for two normalized, non-lateral
derived projections required by the frozen clients: the QGIS/OGR empty-index
projection over `pg_index`, and OGR's default-expression projection over
`pg_attrdef`. Arbitrary derived tables, changed projection expressions, CTEs,
set operations, lateral correlation, implicit join columns, and user/catalog
mixing remain rejected. Actual pgwire against the pinned identity runtime now
executes QGIS `attribute_structure` and OGR `column_structure` directly from the
captured fixtures. It verifies PostgreSQL OID/name/char/text wire types,
default/comment values, and NULL uniqueness from the truthfully empty `pg_index`;
role and legacy filtering still removes hidden defaults, comments, and indexes.
OGR's captured `primary_key_columns` query also executes with its PostgreSQL
`name`/`int2`/`bool` description and returns zero rows, so direct discovery can
select its no-FID path without QuackGIS fabricating a primary key.
This does not claim primary/unique keys, richer index semantics, or authoritative
geometry subtype/dimension/SRID metadata.

The pinned identity lane executes psql 18.3's complete captured 12-stage `\d+`
workflow. QuackGIS structurally rewrites only anchored literal
`OPERATOR(pg_catalog.~)` comparisons carrying `COLLATE pg_catalog.default`; other
custom operators, collations, or unanchored patterns fail with `0A000`. The result
retains exact OID/name/name wire types. Exact `relation_properties` and
`column_properties` shapes read maintained `pg_class`/`pg_am`/`pg_attribute`
state. Foreign-key, policy, extended-statistics, publication, and inheritance
stages return typed empty results only for their exact captured shapes; modified
or general catalog queries still fail closed. The packaged psql gate renders the
copied table's three columns and truthful `ducklake` access method.

## Compatibility surfaces

The profile grows in dependency order. A relation is supported only when its
maintained columns, PostgreSQL result types, cross-references, visibility, and
errors have executable coverage.

### Foundation catalog

| Surface | Purpose |
|---|---|
| `pg_namespace` | schema identity, ownership, and `regnamespace` resolution |
| `pg_class` | tables/views and relation identity |
| `pg_attribute` | ordered columns, nullability, type identity, source attribute numbers |
| `pg_type` plus the required `pg_range` relation shape | built-in, geometry/geography, and scalar resolution; rows exist only for range types explicitly supported later |
| `pg_database` | current logical database identity |
| `current_database`, `current_schema`, `current_schemas` | session discovery and search path |
| PostgreSQL 18.4 startup parameters, `version`, `pg_is_in_recovery`, `SHOW server_version[_num]` | coherent client version selection and truthful local-primary recovery state |
| `oid`, `regclass`, `regtype`, `regnamespace`, `regrole` | symbolic object lookup and catalog joins |
| `format_type`, `to_regclass`, `to_regtype` | client-neutral type and object discovery |

`pg_catalog` must participate in name resolution with PostgreSQL search-path
semantics, including safe handling of unqualified catalog functions and relations.

### Structural discovery

| Surface | Purpose |
|---|---|
| `pg_constraint`, `pg_index` | primary/unique/foreign keys and relationships |
| `pg_attrdef` | supported defaults/generated-column metadata |
| `pg_description` | table/column/routine comments |
| `pg_enum` | enum discovery when QuackGIS supports an equivalent type contract |
| `pg_depend` | only maintained dependency relationships; no invented PostgreSQL storage dependencies |
| `information_schema.schemata` | privilege-aware schema discovery |
| `information_schema.tables`, `columns` | portable, privilege-aware relation/column discovery |
| key/constraint information-schema views | client and REST relationship discovery |
| table/column privilege views | portable role-aware API discovery |
| `pg_table_is_visible`, `pg_relation_is_updatable` | search-path and mutation capability queries |
| `geometry_columns`, `spatial_ref_sys` | bounded PostGIS-compatible geometry/CRS discovery |

The pinned identity lane implements the table/column subset of
`pg_attrdef` and `pg_description` plus `pg_get_expr`, `col_description`, and
`obj_description`. DuckDB/DuckLake remains authoritative for values; durable
relation/attribute mappings supply PostgreSQL identity; `adbin` advertises OID
194 (`pg_node_tree`) and helper results advertise OID 25 (`text`). Content-bearing
rows/helpers bind effective and session roles at the structural rewrite and
intersect common table visibility with the legacy allowlist ceiling. DuckDB's
implicit string `NULL` default marker is normalized to SQL NULL, so explicit
`DEFAULT NULL` is not separately represented. Generated expressions, routine
comments, mutable comment/default DDL over pgwire, and exact PostgreSQL deparser
normalization remain outside this slice.

The next structural slice maps DuckLake's native named `NOT NULL` constraints to
PostgreSQL 18 `pg_constraint` rows with durable registry OIDs, relation/namespace/
attribute references, `int2[]` keys, and `pg_get_constraintdef`. Constraint OIDs
survive table and column rename. `pg_index` has the maintained PostgreSQL wire
shape, including OID 22 `int2vector` identity for `indkey`, but returns no rows;
`pg_get_indexdef` therefore returns NULL. This is intentional: DuckLake 1.x does
not support primary keys, unique keys, foreign keys, check constraints, or
indexes. QuackGIS does not infer a key from a non-null integer or claim uniqueness
it cannot enforce. Constraint/index rows and helper functions use the same
effective-role plus legacy-login metadata ceiling as defaults/comments.

The bounded spatial slice advertises `geometry_columns` and `spatial_ref_sys`
through `information_schema.tables`. `geometry_columns` includes native DuckDB
`GEOMETRY` columns and established QuackGIS WKB geometry-name sentinels, filtered
through the same effective-role/session-login relation ceiling. DuckLake does not
enforce a column-wide subtype, coordinate dimension, or integer SRID, so rows are
deliberately generic `GEOMETRY`, dimension 2, SRID 0. `spatial_ref_sys` preserves
the traced PostgreSQL-compatible column types but is empty until QuackGIS owns an
authoritative CRS registry. `ST_SRID('POINT EMPTY'::GEOMETRY)`, DuckDB-backed
GEOS/PROJ compatibility probes, and textual `ST_Extent`/`ST_3DExtent` execute;
SRID assignment, `Find_SRID`, PostGIS box type identity, and inferred typemods
remain unsupported.

PostgreSQL 18 `has_table_privilege` can report `MAINTAIN`, while
`information_schema.table_privileges` does not list it. The compatibility view
must preserve that distinction rather than dump every internal capability into
the SQL-standard view.

### Roles and privileges

The first implementation is configuration-backed and immutable while the server
is running. This provides real runtime semantics without making role DDL and
credential durability part of the first slice.

Required model:

- LOGIN and NOLOGIN roles;
- authenticated `session_user` and effective `current_user`;
- directed role memberships with cycle rejection;
- PostgreSQL 18 membership-edge `inherit_option`, `set_option`, and
  `admin_option`; initial immutable provisioning keeps `admin_option` false;
- role-level `INHERIT` as the default for newly provisioned membership edges,
  while runtime inheritance follows each edge's `inherit_option`;
- object owner;
- schema `USAGE` and table `SELECT`, `INSERT`, `UPDATE`, `DELETE`, and `MAINTAIN`
  capabilities where the underlying operation exists;
- `PUBLIC` grants where explicitly configured; and
- an authenticator role permitted to assume a bounded set of API roles.

Required PostgreSQL-facing behavior:

- `pg_roles` with no credential disclosure;
- `pg_auth_members` for maintained membership fields;
- `current_user`, `session_user`, and `current_role`;
- `SET ROLE`, `SET LOCAL ROLE`, and `RESET ROLE`;
- `pg_has_role`;
- `has_schema_privilege`, `has_table_privilege`,
  `has_any_column_privilege`, and `has_column_privilege`; and
- PostgreSQL SQLSTATEs for unknown roles, denied role assumption, denied objects,
  and failed transactions.

`SET ROLE` reachability is evaluated from the original `session_user` through
membership edges with `set_option`; changing the current role must not expand the
set of roles the session can subsequently assume. `SET ROLE NONE` restores
`session_user`. `RESET ROLE` restores the connection-time role default; the
initial profile does not support role/database default-role settings, so that
default is `session_user`. Neither reset form requires a membership edge.

Existing read/write/maintenance environment settings migrate into equivalent
implicit grants during a compatibility period. They remain an outer ceiling until
operators have an explicit migration path; they must not silently widen access.
PostgreSQL COPY authorization is preserved: COPY TO derives from `SELECT`, and
COPY FROM derives from `INSERT`, with QuackGIS's bulk-ingest policy remaining an
additional operational ceiling rather than an invented catalog privilege.

Mutable `CREATE/ALTER/DROP ROLE` and `GRANT/REVOKE` come later. They require a
transactional control-metadata format, credential rotation, backup/restore,
audit, upgrade, and administrative authorization before they are safe to expose.

### Request context

The REST authenticator flow requires a bounded PostgreSQL-compatible subset:

```sql
BEGIN;
SET LOCAL ROLE api_reader;
SELECT set_config('request.jwt.claims', $1, true);
-- application or discovery query
COMMIT;
```

Initial rules:

- only explicitly allowed request namespaces are accepted;
- request values are transaction-local and byte-bounded;
- arbitrary DuckDB settings are not exposed through this path;
- request claims are data, never SQL;
- role authorization depends on the validated role mapping, not an untrusted claim;
- direct possession of the authenticator credential is equivalent to the ability
  to forge request context and is therefore a privileged service secret; and
- cleanup is tested across success, error, rollback, cancellation, disconnect,
  and connection reuse.

### Row-level security

RLS is not part of the initial role/catalog slice. It is a separate security
milestone because every maintained read and write shape must be proven against
bypass.

Before an RLS claim, QuackGIS needs:

- a versioned policy representation and ownership/administration rules;
- `USING` and `WITH CHECK` semantics for the supported operation set;
- structural predicate injection at the trusted AST boundary;
- request-context access through bounded settings;
- policy composition for memberships and multiple policies;
- `pg_policy`, `pg_class.relrowsecurity`, and `row_security_active` consistency;
- adversarial coverage for aliases, joins, subqueries, CTEs, views, COPY,
  prepared statements, UPDATE, DELETE, and unsupported statement shapes; and
- exact DuckDB predicates retained in execution plans.

Until those gates pass, documentation and OpenAPI may claim table/operation RBAC,
not PostgreSQL RLS.

## Execution architecture

The implementation should remain small and use DuckDB for relational evaluation:

1. Read authoritative user-object metadata through DuckDB/DuckLake metadata
   functions and information schema.
2. Combine it with QuackGIS role, grant, compatibility identity, and spatial type
   metadata into an immutable `CatalogSnapshot`.
3. Expose snapshot rows through protected internal relations or another bounded
   DuckDB-readable mechanism.
4. Structurally map maintained `pg_catalog` and `information_schema` references to
   those relations; do not recognize whole client query strings.
5. Rewrite session-dependent expressions from trusted pgwire session state or
   evaluate them through the common authorization engine.
6. Let DuckDB execute catalog joins, filters, grouping, and ordering.
7. Attach origin OID/attribute metadata to pgwire RowDescription.
8. Invalidate per-session and REST caches by schema/security epoch.

The implementation may optimize known query families only after arbitrary
relational queries over the maintained catalog rows behave correctly. No branch
may inspect a client product name.

## REST target

The HTTP edge advances in three bounded stages.

### Stage 1: catalog-backed read API

- Replace direct DuckDB `information_schema.columns` assumptions with the
  maintained PostgreSQL catalog/schema provider.
- Keep explicit REST table exposure as an outer ceiling.
- Continue authenticated GET/HEAD, filtering, ordering, and pagination through
  pgwire.

### Stage 2: role-aware OpenAPI

- Validate JWTs at the HTTP trust boundary.
- Map a bounded role claim to a configured assumable database role.
- Use one authenticator pgwire identity and transaction-local role switching.
- Generate paths, methods, columns, and types from the effective role's catalog
  and privilege answers. Relationship embedding remains Stage 3.
- Key caches by effective role, catalog/security epoch, and REST exposure config.
- Ensure an omitted OpenAPI operation is also denied by direct HTTP execution.

Table/operation privileges are sufficient for this stage; RLS controls rows and
must not be represented as object invisibility.

### Stage 3: relationships and mutations

- Add key-derived embedding only after constraint catalog behavior passes through
  actual pgwire.
- Add POST/PATCH/DELETE only after PostgreSQL privilege enforcement, cancellation,
  transaction outcomes, maintained bbox invariants, and representation-return
  semantics are common to direct pgwire and HTTP.
- Add RLS-protected HTTP operations only after the independent RLS milestone.

Full PostgREST RPC, media-type, preference, and differential parity can grow from
real demand; it is not a prerequisite for the common role/catalog boundary.

## Ordered delivery plan

These stages decompose M3/M5 dependencies; they do not supersede the milestone
ordering or exit gates owned by [../ROADMAP.md](../ROADMAP.md).

### C1 — freeze the compatibility contract

Deliver:

- a PostgreSQL 18 profile manifest listing maintained relations, columns,
  functions, casts, settings, and deliberate divergences;
- normalized reference outputs from PostgreSQL 18 plus PostGIS where applicable;
- captured query traces from psql, psycopg, OGR copied-data discovery, headless
  QGIS, and the selected REST schema-cache path; and
- a classification of each observed query as required, safely adaptable, or
  explicitly unsupported.

Gate: every implementation item has a client/workload reason and a PostgreSQL
oracle before compatibility code is added.

Current progress: `pg18-column-core-v1` freezes the first result-type/query
contract and PostgreSQL 18.4 oracle. A real OGR 3.11.5 `-ro -so` copied-point run
against digest-pinned PostgreSQL 18.4/PostGIS freezes 21 ordered session, catalog,
spatial metadata, count, and extent query families plus the exact observed layer
result. A real psql 18.3 `\d+` run freezes 12 more namespace, relation, attribute,
index, constraint, policy, publication, statistics, and inheritance query families
plus its rendered five-column spatial table structure. A real offscreen QGIS
3.44.11 PostgreSQL provider run freezes 32 statements (26 unique families) for
layer open, fields, CRS, privilege/ownership inquiry, count, extent, and a binary
cursor read, with exact successful observed output. The named read-client trace
corpus is closed; C1 remains open only for the role/privilege/OpenAPI reference
matrix that depends on C4/C5 semantics.

### C2 — build catalog snapshot and identity foundations

Deliver:

- authoritative metadata extraction from DuckDB/DuckLake;
- a protected immutable catalog snapshot with schema epoch;
- the durable-object-ID feasibility result and selected OID lifecycle;
- bootstrap role identities and OIDs sufficient for every published owner
  reference before full role/session behavior exists;
- reserved OID ranges and collision/restart/rename tests; and
- atomic invalidation after committed DDL.

Gate: restart and supported rename preserve published identities, rollback
publishes no catalog change, and every nonzero reference selected by the profile
resolves.

Current progress: durable table/column identity and name-reuse behavior pass in
independent DuckDB 1.5.4 processes. The registry and extraction decisions are
closed. The tracked, source/artifact-pinned 1.5.4 lane closes the C2 implementation
gate with transactional OID/attribute mapping, guarded committed snapshots,
commit/rollback epoch behavior, rename/reopen stability, collision checks, and
atomic prepared-read invalidation. QuackGIS owns this patch and its bundle gates;
upstream acceptance is now a deletion opportunity rather than a release gate.

### C3 — implement core catalogs and wire identity

Deliver:

- foundation `pg_namespace`, `pg_class`, `pg_attribute`, `pg_type`, and database
  surfaces;
- minimal `pg_roles` identity rows required to resolve object owners, without yet
  claiming membership, role switching, or privilege semantics;
- search-path and `reg*` resolution;
- PostgreSQL-compatible result types and stable errors;
- source relation OID/attribute numbers in RowDescription; and
- replacement of exact whole-query `pg_type` interception with structural catalog
  behavior.

Gate: client-neutral differential fixtures and actual pgwire tests pass for
scalar, geometry, and geography columns, including restart and rename.

Current progress: the C3 implementation gate is complete in the supported pinned
identity lane. Exact whole-query interception is removed. Explicit and implicit
`pg_catalog` namespace/database/type/range/collation/owner-role references map
structurally to protected process-local views. The stable single logical-database
row and structurally rewritten `current_database`, `current_schema`, and
`current_schemas` functions agree on `quackgis`, `public`, and the maintained
implicit search path, with PostgreSQL `name`/`name[]` result types. Twenty-eight
rows cover 24 exact PostgreSQL 18
profile/QGIS built-ins plus geometry/geography scalars and their PostGIS-shaped
array partners; all array links resolve, spatial delimiters are `:`, and the
`default`/`C` collation OIDs close nonzero references. The custom-type resolver,
ordinary scans, unknown OIDs, namespace/owner identity, OID parameters, and frozen
result types pass through pgwire, including caller-supplied OID parameter types.
Provenance-bound wire hints prevent output aliases from coercing values. Any
unimplemented explicit `pg_catalog` or unqualified `pg_*` relation fails `0A000`
instead of falling through to DuckDB/user objects. CTE shadowing, wildcard,
nested/set/derived type-preserving expressions, `USING`/`NATURAL` catalog joins,
and three-part qualification fail closed until their provenance can be represented.
Direct private-schema access and the structurally lossy `TABLE` query form are
rejected. In the supported pinned identity lane, current DuckLake base
tables now add registry-backed namespace, `pg_class`, `pg_attribute`, and
composite row-type rows. Pgwire joins prove PostgreSQL OID/`name`/internal-`char`
types, scalar/spatial type references, nullability, rename/reopen, retained
attribute gaps, drop/recreate, and non-public schemas. Direct qualified or unambiguous base-table columns and plain wildcard
projections carry matching relation OID/attribute-number origins, including
joins; expressions carry zero origins. The maintained search path resolves
quoted/unquoted, qualified/unqualified `regclass`, `regtype`, `regnamespace`, and
`regrole` values. Strict casts/functions return PostgreSQL `42P01`, `42704`, or
`3F000`; `to_reg*` returns NULL; OID/text casts, bound text input, aliases, arrays,
typmods, and `format_type` have explicit pgwire types and lifecycle tests. Actual
descriptions for every foundation catalog are checked against the client-neutral
`pg18-column-core-v1` fixture. Unsupported column types, malformed/private
functions, complex provenance outside the maintained shape, and signed-only
startup without identity fail closed. The later bounded C5 structural slices
are described below; authoritative spatial typemod/CRS metadata, broader
privilege-aware structure, and expression provenance remain later M3 work, while
key/index semantics require
upstream DuckLake support or an explicit client limitation policy.

All startup auth modes advertise the frozen PostgreSQL 18.4 profile. Structural
`version()` identifies QuackGIS/DuckDB without claiming PostgreSQL execution,
`pg_is_in_recovery()` truthfully returns PostgreSQL `bool` false for the local
primary, and `SHOW server_version`/`server_version_num` agree. Unsupported
arguments/window forms fail closed.

The exact QGIS 3.44 four-statement session bootstrap now passes as the only
multi-statement exception: simple-protocol batches are limited to eight structurally
parsed, individually allowlisted `SET` statements. `extra_float_digits=3`,
`datestyle=ISO`, a control-free 64-byte `application_name`, and maintained
`client_min_messages` values are accepted without widening general SQL batching.

### C4 — implement role and session semantics

Deliver:

- configuration schema for LOGIN/NOLOGIN roles, membership-edge options,
  assumable roles, owners, and grant declarations consumed by C5;
- session/authenticated/effective identity state;
- `SET ROLE`, `SET LOCAL ROLE`, `RESET ROLE`, and identity expressions;
- bounded transaction-local request context; and
- cleanup across every transaction/cancellation/reuse outcome.

Gate: direct pgwire role/session tests prove role assumption follows PostgreSQL 18
membership-edge semantics and no effective identity or request context leaks
across transactions, sessions, cancellation, or reused native connections.

Current progress: C4 is complete. A bounded immutable JSON schema
with explicit stable role OIDs, LOGIN/NOLOGIN and INHERIT flags, PostgreSQL 18
membership-edge options, table owners, and the schema/table grant vocabulary
consumed by C5. Startup rejects trust mode, LOGIN/auth mismatches, duplicate or
reserved identities, unknown principals, duplicate edges, cycles,
`admin_option=true`, unsupported privileges/schemas, unknown fields, and input
over 1 MiB. Set-option reachability is evaluated from the original login role and
is independent of configuration order. Per-connection state now exposes
PostgreSQL `name`-typed `session_user`, `current_user`, `current_role`, and `user`,
and supports session/local role assumption, `NONE`, and reset with `42704`/`42501`
errors. Local identity is removed after commit, rollback, and failed-transaction
rollback; independent connections remain isolated. Simple and extended idle
`COMMIT`/`ROLLBACK` are harmless, failed `COMMIT` reports rollback, a following
QGIS-style `ROLLBACK` remains harmless, and unsupported work in a failed
transaction returns `25P02` before compatibility-shape errors. A role/context epoch rejects
prepared execution after an identity change rather than reusing stale
authorization. C5 now consumes those declarations for non-widening statement
authorization, catalog ownership, and privilege inquiry. The exact
transaction-local `set_config('request.jwt.claims', $1, true)` and
`current_setting(..., true)` path now stores bound text only at the pgwire session
edge, allows one 16 KiB setting under a 32 KiB aggregate ceiling, rejects NUL and
arbitrary/non-local settings, returns PostgreSQL `text`, invalidates stale
prepared work, and clears on commit/rollback/failed-transaction rollback. The
actual pgwire cancellation case sets local role/claims before cancelling a stream,
proves failed-transaction and native quarantine prevent reuse or context reads,
and proves a fresh connection has its session-user identity with no claims.
Privilege integration, inquiry, and role catalogs are implemented in C5.

### C5 — implement privilege and discovery semantics

Deliver:

- the common authorization engine for schema/table/column/operation checks;
- role and membership catalogs;
- privilege inquiry functions;
- structural constraints, comments, keys, and maintained information-schema
  views; and
- compatibility translation from existing allowlists to non-widening grants.

Gate: catalog inquiry, information-schema visibility, OpenAPI eligibility, and
actual statement authorization agree for anonymous, reader, editor, and denied
roles.

Current progress: the common enforcement core is active. One pure role-catalog
decision combines schema USAGE, table ownership, direct grants, inherited grants
over `inherit_option=true` edges, and `PUBLIC`. It gates maintained SELECT,
INSERT, UPDATE, DELETE, COPY FROM/INSERT, MAINTAIN, and predeclared-owner CREATE
before DuckDB while preserving legacy allowlists as an outer ceiling. Actual
SCRAM pgwire cases prove an assumed role without SELECT is denied, a SELECT-only
role can read, and that role cannot inherit the login's legacy write access. Role
and membership catalogs are now ordinary protected relational views:
`pg_roles` publishes PostgreSQL-shaped non-superuser capability fields with NULL
password/expiry/config values, while `pg_auth_members` publishes explicit stable
edge OIDs, role/member/grantor references, and admin/inherit/set options. Actual
pgwire joins prove all references resolve to role rows and no credentials appear.
Configured owners project into durable-identity `pg_class.relowner`.
`pg_has_role`, `has_schema_privilege`, `has_table_privilege`,
`has_any_column_privilege`, and `has_column_privilege` call the same Rust role
decisions used by execution. Bounded comma-separated privilege literals cover the
PostgreSQL 18 keywords, `WITH GRANT OPTION`/`WITH ADMIN OPTION` correctly report
false for immutable grants, and role inquiry distinguishes MEMBER, immediately
inherited USAGE, and SET reachability. Name-literal object inquiry passes the
official DuckLake SCRAM workflow; OID/catalog-expression inquiry and exact column
existence resolve through durable catalog identity and fail closed without it.
Actual pgwire cases prove writer ownership, PUBLIC schema usage, ungranted denial,
and SELECT-only inquiry agree with allowed SELECT and denied INSERT. Role-aware
`information_schema.schemata`, `tables`, `columns`, `table_privileges`,
`role_table_grants`, `column_privileges`, and `role_column_grants` bind the
effective role at the structural pgwire edge, derive object/column existence from
DuckDB, and advertise PostgreSQL 18 `name`/`varchar` result identity. Table
visibility intersects the common schema-USAGE/table-operation decision with the
legacy identity/allowlist ceiling; owner, direct, inherited, and PUBLIC cases pass
actual pgwire alongside ungranted denial. Standard privilege views omit
QuackGIS-only `MAINTAIN`, expand eligible table grants across columns, exclude
PUBLIC from the `role_*` variants, and report immutable grants as non-grantable.
Role changes invalidate prepared discovery statements rather than retaining a
stale role literal. The first shared traced structural slice now projects
DuckLake defaults and table/column comments through `pg_attrdef`,
`pg_description`, `pg_get_expr`, `col_description`, and `obj_description`.
Column type/default/comment/nullability/constraint-name values participate in the
guarded catalog fingerprint epoch; actual pgwire checks durable relation/attribute
references, exact PostgreSQL `oid`/`int2`/`pg_node_tree`/`text` result identity,
owner and SELECT-granted visibility, and legacy-allowlist denial despite a table
grant. Durable NOT-NULL constraint rows and definitions now carry PostgreSQL
`name`, internal `char`, `int2[]`, and OID references; rename continuity passes.
The maintained `pg_index`/`pg_get_indexdef` shape returns no rows/definitions in
agreement with DuckLake. Generic spatial catalog/version/SRID/extent shapes pass
focused actual pgwire with role and legacy filtering. Authoritative CRS/subtype/
dimension metadata, generated-column semantics, wider traced query shapes, and
any future upstream key/index support remain open C5 work.

### C6 — qualify named clients

Deliver:

- copied-data psql and psycopg discovery workflows;
- OGR read and COPY workflows without optional discovery failure;
- headless QGIS discovery, filter, identify, and render;
- stable subtype/SRID/dimension metadata required by those clients; and
- query-shape regression fixtures independent of client-name branches.

Gate: all Local 1.0 named clients pass in the pinned Kind topology and the same
catalog tests pass directly on the host.

Current progress: pinned psycopg 3.2.13 passes one copied-data workflow in the
minimal Kind topology through the mutual-TLS tiny client. It creates/reuses a
client-neutral official-DuckLake table, clears it, streams exact WKB and NULL rows
with PostgreSQL text COPY, closes and reconnects, and verifies exact scalar and
spatial readback. Pinned GDAL/OGR 3.11.5 then reads that same fixture through its
unmodified extended-protocol SQL-result cursor lifecycle (`BEGIN`, `DECLARE`,
`FETCH 0`, bounded `FETCH`, `CLOSE`, transaction end) and must produce exact
GeoJSON for `POINT (1 2)` plus NULL geometry/property values. Its direct-discovery
path now passes the same exact rows with truthful no-FID behavior. OGR also
appends a Point/NULL GeoJSON fixture to a separate predeclared table with
`PG_USE_COPY=YES`; the bounded decoder accepts its plain PostGIS EWKB hex only on
classified spatial fields, and a fresh connection proves exact atomic
publication. Psql 18.3 runs the full captured `\d+` workflow and reports
`ducklake`; psycopg and OGR also pass again after ordered Pod replacement and
mTLS/iroh key rotation with old-client denial. OGR-created tables and
authoritative CRS metadata remain open.

Exact offscreen QGIS 3.44.11-Solothurn now passes the optional digest-pinned Kind
gate as a read-only copied-data query layer with an explicit `id` key. It discovers
exact `id`/`name` fields, counts two rows, and reads Point/NULL features through
the binary cursor. The pinned lane executes its full layer privilege projection
with PostgreSQL `bool` `pg_is_in_recovery=false`, while all startup modes advertise
PostgreSQL 18.4 and `version()`/`SHOW server_version[_num]` agree.
Simple and extended idle transaction end, failed `COMMIT`-as-rollback, subsequent
`ROLLBACK`, explicit failed `ROLLBACK`, and `25P02` precedence pass without a
client-name branch. Native actual-pgwire coverage now executes the exact captured
QGIS read-only/binary cursor start, `FETCH FORWARD 2000`, and close/commit shapes;
it verifies binary raw WKB/BIGINT/text/NULL values, close/rollback, failed-declare
status, and read-only `25006` cleanup. The pinned identity lane also executes the
captured QGIS `attribute_structure` query with exact default/comment/empty-index
semantics. Direct ordinary-table open remains blocked on real primary/unique-key
support; broader filter, identify, extent, and render workflows remain C6 work.

### H1 — migrate and package role-aware REST

Deliver:

- catalog-backed schema discovery;
- JWT verification and bounded role mapping;
- authenticator plus transaction-local role/context flow;
- role-aware OpenAPI and cache invalidation;
- immutable REST image and multiple-replica Kind deployment; and
- denial, credential rotation, readiness, and load-balancing tests.

Gate: two REST replicas produce the same role-specific API, cannot exceed the REST
exposure ceiling or database grants, and continue to exercise only pgwire.

Current progress: H1 is complete at the K0 package boundary. Direct mode validates
bounded HS256 JWTs, exact issuer/audience/time/role, role-filtered discovery,
transaction-local role/claims cleanup, exact cache fallback, shared epochs where
durable identity exists, and owner-only JWT/authenticator rotation. Packaged mode
uses no database password: bootstrap derives `authenticator` from a distinct
proven service credential, the worker binds the pgwire startup user to that
lease, and the server accepts only configured `LOGIN` roles on its loopback
edge-preauthenticated listener before `AuthenticationOk`.

Two REST Pods each run a loopback tiny client with a unique transport key. Both
independently return the same exact reader data/OpenAPI, hide and deny an ungranted
role, reject missing JWTs and writes, and publish two ready EndpointSlice
addresses. Deleting one Pod leaves the HTTP Service usable. A stable internal UDP
Service and bounded stale-session invalidation recover both replicas after a
3.935-second core replacement. Packaged mTLS/edge rotation denies the old client
certificate and old authenticator credential's next lease; a separate JWT
replacement denies an old token against each replacement Pod. Signed shared
epochs, public HTTP TLS/rate policy, zero-downtime multi-key overlap, full
PostgREST behavior, and RLS remain follow-on work.

### C7 — add relationships and broader PostgreSQL structure

Deliver only surfaces justified by the next HTTP/client behavior:

- foreign-key relationship discovery and embedding;
- supported routine metadata and execution privileges if RPC is selected;
- enums/ranges and generated-column behavior; and
- richer count, representation, CSV, singular, and GeoJSON paths.

Gate: each feature has a reference PostgreSQL/PostgREST case and an actual
QuackGIS-pgwire case.

### S1 — add structural RLS

Deliver the RLS requirements above as one security-reviewed slice before exposing
RLS-protected mutations.

Gate: the adversarial read/write bypass suite passes, catalog claims agree with
execution, unsupported shapes fail before native execution, and exact predicates
remain in DuckDB plans.

### A1 — add mutable SQL administration

Only after control metadata has production durability:

- transactional role and grant DDL;
- credential lifecycle and revocation;
- protected administrator roles;
- audit events;
- backup/restore and upgrade migration; and
- concurrent change/cache invalidation tests.

Supported role rename must retain the role OID; until this stage, configured role
identity is required to survive restart and configuration reordering, not rename.

Gate: role/grant changes are atomic, recoverable, audited, and cannot widen access
through partial failure or stale caches.

## Local 1.0 commitment

Local 1.0 requires C1 through C6 and H1 at a bounded scope:

- PostgreSQL 18 compatibility profile for the named clients;
- coherent core catalog and RowDescription identity;
- configuration-backed roles, memberships, table/operation privileges, and role
  switching;
- bounded transaction-local REST request context;
- role-aware read/OpenAPI HTTP deployment;
- explicit REST exposure ceiling; and
- packaged rotation, recovery, upgrade, and soak evidence for those capabilities.

Local 1.0 does not require mutable role DDL, PostgreSQL RLS, REST mutations/RPC,
all PostgreSQL catalogs, or full PostgREST parity. Those remain ordered follow-on
work rather than shortcuts inside the initial authorization boundary.

## Verification matrix

Every supported surface needs evidence at the appropriate rings:

| Ring | Required evidence |
|---|---|
| unit/property | role graph cycles, privilege closure, OID allocation/collision, identifier quoting, request-size bounds |
| PostgreSQL differential | normalized rows, result types, nullability, visibility, SQLSTATE, `reg*` resolution; dynamic numeric OIDs excluded from equality |
| actual pgwire | simple/extended protocol, parameters, RowDescription origins, transactions, cancellation, failed-state behavior |
| cross-surface consistency | privilege inquiry versus execution versus information schema versus OpenAPI |
| named client | captured copied-data workflow with pinned version and no client-name branch |
| lifecycle | DDL/role epoch invalidation, restart, rename, backup/restore, upgrade, rotation |
| adversarial security | secret catalog denial, role escalation, context leakage/forgery, stale cache, policy bypass |
| packaged topology | authenticator and client secrets, multiple REST replicas, readiness, load balancing, drain/restart |

Catalog tests compare semantics rather than installation-specific OID numbers. A
supported catalog query must execute as an ordinary relational query over the
maintained surface; matching one exact SQL serialization is not sufficient.

## Pragmatic risk controls

| Risk | Control |
|---|---|
| compatibility scope explosion | PostgreSQL 18 profile manifest plus captured-query admission; no surface without a maintained consumer |
| second schema authority | derive user objects from DuckDB/DuckLake; limit QuackGIS persistence to control metadata and compatibility identity |
| unstable object identity | feasibility gate before exposing relation OIDs; no DuckDB transient OIDs or name-only hashes |
| metadata/enforcement drift | one authorization engine and cross-surface consistency tests |
| credential disclosure | no credentials in `pg_roles`, logs, metrics, OpenAPI, or REST errors; restrict sensitive catalogs |
| stale role/schema caches | monotonic epochs, transaction-aware publication, fail-closed invalidation |
| unsafe RLS shortcut | table/operation RBAC first; separate structural RLS milestone and bypass suite |
| PostgREST backend gravity | reuse protocol-neutral parser/schema pieces; keep PostgreSQL-facing semantics in QuackGIS and all data access over pgwire |
| false PostgreSQL claims | publish the exact profile and divergences; unsupported behavior returns stable errors |
