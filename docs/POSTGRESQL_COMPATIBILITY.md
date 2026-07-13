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

`quackgis-rest` remains a stateless pgwire client. It must not own an independent
schema cache authority, role model, or row-policy implementation. Its exposure
configuration is an additional HTTP ceiling, never a replacement for database
privileges.

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

The current runtime has only a bootstrap contract:

- DuckDB-derived `information_schema.columns` supports the read-only REST preview;
- `public` relation names are structurally mapped to DuckDB/DuckLake `main`;
- geometry and geography use two maintained pgwire type OIDs;
- process-local relational `pg_namespace`, `pg_type`, and `pg_range` views expose
  the two spatial OIDs through structurally rewritten explicit catalog references;
- RowDescription and text/binary/NULL WKB transport are tested; and
- broad `pg_catalog` and restricted-user metadata access fail closed.

This does not yet provide user-object PostgreSQL catalogs, stable relation OIDs,
source relation/attribute identity in RowDescription, PostgreSQL roles, ACLs,
role switching, or role-aware OpenAPI. The spatial type views are the first
relational C3 slice, not a broad catalog claim.

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
- QuackGIS-owned control metadata may store roles, memberships, grants, policy,
  catalog epochs, and compatibility identity mappings. It must be protected,
  versioned, backed up with the logical catalog, and written only through the
  supported DuckDB/DuckLake transaction path.
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

A reference implementation based on upstream DuckLake commit
`d4a23e83cab5ff81d239a40c7891141c19c611cb` passes its focused lifecycle,
transaction, nesting, view, and exact-schema tests plus the complete upstream
function-test group. This is proposal evidence, not QuackGIS runtime evidence:
the function is not yet merged or present in the pinned DuckDB 1.5.4 bundle.
C2 must not consume it until the public contract is accepted upstream and a
version-pinned official extension reproduces those tests. Private attachment
names must not leak into client SQL.

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
independent DuckDB 1.5.4 processes. The registry and lower-maintenance extraction
decisions are closed, and the upstream public-function proposal passes against
DuckLake main. C2 remains open for upstream acceptance and a pinned-runtime
contract test, transactional OID/attribute mapping, immutable snapshot, and
commit/rollback epoch behavior.

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

Current progress: exact whole-query interception is removed. Explicit and implicit
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
rejected. User-object catalogs/OIDs, broader built-ins, `reg*`, and RowDescription
relation/attribute origins remain open.

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

### C6 — qualify named clients

Deliver:

- copied-data psql and psycopg discovery workflows;
- OGR read and COPY workflows without optional discovery failure;
- headless QGIS discovery, filter, identify, and render;
- stable subtype/SRID/dimension metadata required by those clients; and
- query-shape regression fixtures independent of client-name branches.

Gate: all Local 1.0 named clients pass in the pinned Kind topology and the same
catalog tests pass directly on the host.

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

### C7 — add relationships and broader PostgreSQL structure

Deliver only surfaces justified by the next HTTP/client behavior:

- foreign-key relationship discovery and embedding;
- supported routine metadata and execution privileges if RPC is selected;
- enums/ranges/defaults/generated-column behavior; and
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
