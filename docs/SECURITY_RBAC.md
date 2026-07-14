# Security and authorization

QuackGIS implements immutable configuration-backed PostgreSQL table/operation
RBAC behind a separate service-identity and table-allowlist ceiling. It does not
implement PostgreSQL RLS or mutable role/grant administration. The target is the
bounded PostgreSQL 18 role, privilege, session, catalog, and request-context
contract in [POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md).

Target design is not current evidence. This document separates the implemented
floor from the ordered security work.

## Implemented controls

| Control | Evidence |
|---|---|
| development trust mode | local only; explicit status/logging |
| SCRAM-SHA-256 password startup | real DuckDB pgwire workflow |
| read/write and optional read-only identity | auth unit + real workflow |
| normalized read/write table allowlists | structural policy unit + denied real cases |
| separate opt-in maintenance identity and table policy | auth/parser unit + real pgwire compaction workflow |
| fail-closed TLS material configuration | startup validation |
| explicit TLS-required policy | actual-process encrypted-client and plaintext-denial profile |
| certificate/password rotation | actual-process restart profile rejects old trust/password and preserves exact state |
| bounded/redacted auth and authorization audit events | audit/policy tests |
| optional private metrics endpoint | metrics unit tests |
| native driver digest/version validation | storage unit/native tests |
| bounded immutable role configuration and graph validation | auth/role unit tests |
| session/effective identity, role switching, and transaction-local cleanup | real pgwire workflow + role units |
| bounded transaction-local `request.jwt.claims` context | real pgwire workflow + role/parser units |
| common schema/table privilege enforcement | role/policy units + real pgwire role-denial/grant cases |
| role-aware schema/table/column/grant discovery | real pgwire workflow + catalog/parser units |

The role configuration parser validates stable explicit OIDs, LOGIN/NOLOGIN,
INHERIT defaults, PostgreSQL 18 membership-edge options, cycles, owners, and the
bounded schema/table privilege vocabulary. Sessions expose PostgreSQL `name`
identity for `session_user`, `current_user`, `current_role`, and `user`; implement
`SET ROLE`, `SET SESSION ROLE`, `SET LOCAL ROLE`, `SET ROLE NONE`, and
`RESET ROLE`; and clear local role state on commit/rollback. Privilege inquiry
and role-aware schema/table/column/grant discovery are also implemented. RLS,
role-aware OpenAPI, administrative SQL, packaged secret rotation, revocation
infrastructure, and production failure drills remain open.

## Trust boundaries

1. **Client → pgwire:** use SCRAM and `QUACKGIS_TLS_MODE=required` outside local
   development. `preferred` mode permits plaintext for development; required mode
   needs paired certificate/key material and rejects insecure startup before auth.
2. **HTTP client → REST:** the current bearer token is a preview control. The
   target validates JWT signature, issuer, audience, time bounds, and a bounded
   role claim before opening a database transaction. HTTP authentication does not
   itself authorize a database object.
3. **REST → pgwire:** the target authenticator credential is a privileged service
   secret because its holder can assume configured API roles and set request
   context. REST never receives ADBC or storage access.
4. **SQL → policy → ADBC:** exactly one general structurally parsed statement is
   authorized before prepare/schema access. The bounded all-allowlisted session
   `SET` batch is handled without ADBC; unknown write/read/catalog/policy shapes
   fail closed.
5. **Catalog/control metadata:** user schema comes from DuckDB/DuckLake. Protected
   QuackGIS metadata may hold roles, memberships, grants, policy, compatibility
   OIDs, and epochs, but must not become an independent user-schema authority.
6. **Storage:** local data roots carry one authority marker. Remote credentials and
   shared profiles are disabled.
7. **Metrics/audit:** never include SQL text, parameters, request claims, WKB,
   credentials, signed URIs, or sensitive paths.

## Current authorization model

- `QUACKGIS_WRITE_ALLOWLIST` restricts write-capable identities.
- `QUACKGIS_READ_ALLOWLIST` restricts table reads and denies broad metadata access
  until maintained PostgreSQL catalog surfaces exist.
- Object identity is normalized from the parsed statement, never a raw SQL prefix.
- General SQL remains exactly one statement. The only multi-statement exception is
  a simple-protocol batch of at most eight structurally parsed statements where
  every member is an allowlisted session `SET`; mixed or over-limit batches fail
  before authorization/execution and `application_name` is control-free/64 bytes.
- Compatibility rewrites may shape SQL/results but may not change the underlying
  table authorization decision. Direct, joined (including PIVOT/UNPIVOT wrappers),
  derived, CTE, set-operation, and select/query-level expression-subquery reads
  share the same structural target collection; private, explicit, and unqualified
  catalog names cannot bypass the metadata denial. `TABLE` is rejected before
  authorization because sqlparser does not retain an equivalent object identity.
- `QUACKGIS_MAINTENANCE_USER` grants one existing read/write identity the bounded
  compaction call; the write table allowlist still applies. Backup/restore,
  retention, and future shared storage remain offline/operator capabilities and
  do not inherit SQL access.

These settings remain an outer non-widening ceiling during migration. They are not
PostgreSQL grants and must not be reported as such in `pg_catalog` before the
common privilege engine exists.

## Target role model

The first role slice is configuration-backed and immutable during one server run.
That deliberately separates runtime authorization correctness from mutable role
DDL, credential persistence, and administrative recovery.

Each role has:

- a unique normalized name and stable compatibility OID;
- LOGIN or NOLOGIN;
- optional SCRAM credential reference for LOGIN roles;
- role-level `INHERIT`, used as the provisioning default for membership edges;
- direct membership edges with PostgreSQL 18 `inherit_option`, `set_option`, and
  `admin_option` (`admin_option` is false in the initial immutable model);
- owned objects and explicit grants; and
- no implicit superuser, database creation, replication, or RLS-bypass behavior.

The graph must reject duplicate names, unknown members, cycles, privilege names
unsupported by the target object, and an authenticator with unbounded role
assumption.

Every session tracks:

- authenticated login role and `session_user`;
- effective `current_user`/`current_role`;
- session role selected by `SET ROLE`;
- transaction-local role selected by `SET LOCAL ROLE`;
- bounded transaction-local request settings; and
- transaction failure/cleanup state.

Role assumption is evaluated from the original `session_user` through edges with
`set_option`; assuming one role cannot expand the roles selectable by a later
`SET ROLE`. `SET ROLE NONE` restores `session_user`. `RESET ROLE` restores the
connection-time role default, which is `session_user` in the initial profile
because role/database default-role settings are unsupported. Local state
disappears on commit and rollback; all state disappears on disconnect.
Cancellation and quarantine cannot publish or retain a role transition.

## Target object privileges

The initial maintained object model is intentionally smaller than PostgreSQL's
complete ACL system:

| Object | Initial privileges |
|---|---|
| schema | `USAGE` |
| table/view | `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `MAINTAIN` where the operation exists |
| column | `SELECT`, `INSERT`, `UPDATE` only when a maintained client requires it |
| role | membership/inheritance and explicit role-assumption capability |

Ownership, direct grants, inherited grants, and explicitly configured `PUBLIC`
grants feed one pure authorization decision. Unsupported PostgreSQL privileges
must return a stable error rather than silently map to a broader capability.
COPY TO derives from `SELECT`; COPY FROM derives from `INSERT`. QuackGIS may keep
an additional bulk-ingest ceiling, but it is not exposed as a PostgreSQL ACL bit.

The same decision must drive:

- structural statement authorization before native prepare;
- COPY and maintenance authorization;
- `pg_has_role` and `has_*_privilege`;
- privilege-aware `information_schema` views;
- role-aware OpenAPI path/method/column generation; and
- REST request execution.

A cross-surface test must fail if any two answers differ.

The surfaces remain PostgreSQL-specific where required: `has_table_privilege`
may report `MAINTAIN`, but PostgreSQL 18
`information_schema.table_privileges` does not list `MAINTAIN` and QuackGIS must
not add it there.

## Catalog visibility and secrets

PostgreSQL does not globally hide all catalog object names from roles lacking table
access. QuackGIS will therefore implement relation-specific PostgreSQL behavior:

- maintained `pg_catalog` relations expose the PostgreSQL-compatible structural
  rows defined by the selected profile;
- maintained `information_schema` views apply role/ownership filters;
- `pg_roles` never returns a password verifier;
- `pg_authid`, internal role configuration, JWT material, SCRAM verifier storage,
  compatibility OID state, and policy definitions are denied except where an
  explicit future administrator contract requires access; and
- HTTP exposure configuration can omit a resource without changing direct pgwire
  catalog behavior.

Object-name confidentiality, if added, is a deliberate restrictive mode and a
published PostgreSQL divergence—not an accidental consequence of table denial.

## REST authenticator and request context

The target REST flow is:

```sql
BEGIN;
SET LOCAL ROLE api_reader;
SELECT set_config('request.jwt.claims', $1, true);
-- discovery/application query
COMMIT;
```

Security requirements:

- JWT algorithms and keys are operator-configured; token-provided algorithms are
  not trusted.
- Issuer, audience, expiry, not-before, size, and role mapping are validated.
- A claim can select only a statically configured role that the authenticator may
  assume.
- Credentials are never carried in the token.
- Only allowlisted setting namespaces are accepted, values are byte-bounded, and
  initial support is transaction-local only.
- Claims are bound values, never SQL text or identifiers.
- Database authorization is based on the effective role. Claims influence row
  policy only after the independent RLS milestone.
- Context and role reset is tested on success, database error, failed transaction,
  timeout, cancellation, disconnect, and connection reuse.
- OpenAPI caches are keyed by effective role, catalog/security epoch, and REST
  exposure configuration.

Possession of the authenticator database credential permits forged request
context. It therefore receives the same secret handling, rotation, audit, and
network restrictions as a signing key.

## Row-level security boundary

Table and operation RBAC ships before RLS. QuackGIS must not label table allowlists,
grants, REST filters, or bbox injection as row-level security.

RLS requires a separate security-reviewed implementation with:

- versioned `USING` and `WITH CHECK` policy representation;
- owner, membership, policy-composition, and administration semantics;
- structural injection before DuckDB planning for every supported read/write
  shape;
- request-context access only through bounded transaction-local settings;
- consistent `pg_policy`, `pg_class.relrowsecurity`, and
  `row_security_active` behavior;
- fail-closed handling of aliases, CTEs, subqueries, joins, views, COPY, prepared
  statements, UPDATE, DELETE, and unknown AST forms; and
- an adversarial bypass suite proving the exact DuckDB predicate remains present.

REST mutations protected by RLS remain blocked until direct pgwire reads and
writes pass that suite.

## Mutable role and grant administration

`CREATE/ALTER/DROP ROLE` and `GRANT/REVOKE` are later capabilities. Before exposure
they require:

- protected transactional control metadata written through DuckDB/DuckLake;
- atomic role/grant change plus schema/security epoch publication;
- credential creation, expiry, rotation, revocation, and secure backup;
- an explicit administrator role and no ambient superuser;
- redacted audit events;
- concurrent change and stale-cache behavior;
- backup/restore and upgrade migrations; and
- deterministic recovery after response loss or interrupted commit.

Configuration-backed provisioning remains the supported path until those gates
pass.

## Immutable role configuration

`QUACKGIS_ROLE_CONFIG` (or `--role-config`) names a UTF-8 JSON file of at most
1 MiB. Role configuration is accepted only with password authentication, and its
LOGIN roles must exactly equal the configured read/write and optional read-only
authentication users. Role names are bounded lowercase unquoted PostgreSQL
identifiers. Role and membership-edge OIDs are explicit so identity is independent
of document ordering; OID zero and the bootstrap owner OID 10 are reserved.

```json
{
  "roles": [
    {"oid": 100001, "name": "authenticator", "login": true},
    {"oid": 100002, "name": "reader", "login": true},
    {"oid": 100003, "name": "api_reader", "inherit": true}
  ],
  "memberships": [
    {
      "oid": 200001,
      "role": "api_reader",
      "member": "authenticator",
      "inherit_option": true,
      "set_option": true,
      "admin_option": false
    }
  ],
  "table_owners": [
    {"table": "public.places", "role": "authenticator"}
  ],
  "schema_grants": [
    {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
  ],
  "table_grants": [
    {"table": "public.places", "role": "api_reader", "privileges": ["SELECT"]}
  ]
}
```

`inherit_option` defaults from the member role's `inherit` setting and
`set_option` defaults to true. `admin_option=true` is rejected because mutable
role administration is outside this immutable slice. Membership self-edges,
cycles, duplicate edges, unknown roles, duplicate owners/grants, unsupported
schemas/privileges, and unknown JSON fields all fail startup. `PUBLIC` is valid
only as a grant grantee, not as a configured role.

Owner and grant declarations now feed one pure authorization decision. Schema
`USAGE` is required with table access; table ownership supplies every maintained
table capability; direct, inherited (`inherit_option=true`), and `PUBLIC` grants
supply their declared capabilities. The same decision gates SELECT, maintained
INSERT/UPDATE/DELETE, COPY FROM as INSERT, and maintenance as MAINTAIN. Immutable
CREATE is accepted only for a predeclared table owner with schema USAGE. Existing
allowlists remain a non-widening outer ceiling, so the role file cannot broaden a
login's legacy access. `pg_roles` and `pg_auth_members` project the immutable
graph as ordinary protected relational views. They include the bootstrap owner,
all configured roles, stable edge OIDs, resolving role/member/grantor references,
and PostgreSQL 18 membership options. Password/verifier fields are always NULL.
Configured table ownership also supplies `pg_class.relowner` in the durable
identity lane. `pg_has_role`, `has_schema_privilege`, `has_table_privilege`,
`has_any_column_privilege`, and `has_column_privilege` now consume the same role
decisions. They accept PostgreSQL-valid comma-separated text-literal privilege
lists, report immutable grants as non-grantable/non-admin, and preserve MEMBER,
INHERIT/USAGE, and SET edge semantics. Name-literal object inquiry works in the
official lane; OID or catalog-expression object inquiry and exact column
existence require the durable identity lane and otherwise fail with `0A000`.
Column grants are not provisioned, so maintained column inquiry and
`column_privileges` rows derive from matching table privileges. Role-aware
OpenAPI remains open.

With a valid role file, role assumption walks only `set_option=true` membership
edges from the original authenticated `session_user`. A changed `current_user`
does not become a new traversal root. Unknown roles return `42704`; unreachable
roles return `42501`. `SET LOCAL ROLE` is deliberately rejected with `25001`
outside an explicit transaction rather than emitting PostgreSQL's warning/no-op.
Session role survives transaction end, while a local role (including local
`NONE`) is removed after commit, rollback, and failed-transaction rollback.
Identity changes invalidate already prepared statements instead of executing a
statement authorized or rendered under a stale effective role. Disconnect drops
all role state, and independent pgwire connections have independent state.

The maintained request-context surface is exactly:

```sql
BEGIN;
SELECT set_config('request.jwt.claims', $1, true);
SELECT current_setting('request.jwt.claims', true);
COMMIT;
```

The setter accepts one text literal or `$1`, requires the third argument to be
literal `true`, and requires an explicit transaction. One value is limited to
16 KiB, total per-session request context to 32 KiB, and NUL is rejected. Values
remain bound data and are never reparsed as SQL. Only `request.jwt.claims` is
allowlisted; arbitrary names, non-local settings, additional query clauses,
embedded setters, and DuckDB-qualified setting functions fail before native
execution. The getter requires `missing_ok=true`, returns PostgreSQL `text`, and
returns NULL when unset. Commit, rollback, and failed-transaction rollback clear
the value; a change invalidates prepared statements rendered under an older
session epoch. The actual pgwire cancellation oracle sets local role and claims,
cancels a streaming query inside that transaction, proves the failed session
cannot read context or recover its quarantined native connection, then proves a
fresh connection has session-user identity and no claims.

## Required Local 1.0 evidence

Existing requirements remain:

- malformed/half-configured TLS fails startup;
- wrong password never falls back to trust;
- a real encrypted client and plaintext-denial workflow is verified by
  `just duckdb-tls-rotation-profile`;
- current read-only and allowlist denials return stable SQLSTATE `42501` before
  ADBC;
- query timeout/cancel and connection quarantine do not bypass policy;
- restart-based certificate/password rotation has host-process evidence; and
- packaged rotation, revocation, and administrative audit remain required.

The expanded PostgreSQL/RBAC commitment additionally requires:

- role graph and grant validation fails startup without partial service;
- direct pgwire role switching and privilege inquiry agree with actual execution;
- catalog and information-schema visibility match the declared PostgreSQL 18
  profile for denied, anonymous, reader, and editor roles;
- role/request state never leaks across transaction or connection lifecycle;
- role/schema epoch changes invalidate REST and pgwire metadata safely;
- role-aware OpenAPI cannot expose or execute an operation denied by either the
  REST ceiling or database privileges;
- `pg_roles`, errors, metrics, logs, and OpenAPI disclose no credential or raw
  request claim; and
- backup/restore, restart, upgrade, and packaged rotation preserve the exact role,
  membership, grant, OID, and epoch contract.

Do not claim complete PostgreSQL object privileges, RLS, mutable role SQL, or full
PostgREST security. Publish only the selected profile and executable gates.
