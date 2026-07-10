# Security and RBAC hardening plan

QuackGIS' current security surface is intentionally small: pgwire SCRAM password
auth, optional pgwire TLS, a configured read/write user, an optional read-only
user, an optional write-target allowlist for service identities, and fail-closed
write authorization at the DuckLake SQL boundary. Full PostgreSQL RBAC is not
implemented.

This document defines what must be true before widening the security claim.

## Current implemented controls

| Control | Status | Evidence |
|---|---|---|
| Trust-mode local development | ✅ default preview | local smokes |
| SCRAM-SHA-256 password auth | ✅ implemented | `password_auth_and_readonly_role_fail_closed` |
| Read/write vs read-only roles | ✅ coarse role model | AST allowlist permits queries/session/read controls and denies all mutating or indeterminate statement variants before catalog refresh with SQLSTATE `42501`; denials increment `quackgis_write_denied_total` |
| Write service table allowlists | ✅ implemented for write-capable identities | `QUACKGIS_WRITE_ALLOWLIST` / `--write-allowlist` normalizes `table`, `public.table`, `main.table`, and `quackgis.main.table`; denied or indeterminate writes fail with SQLSTATE `42501`; `password_auth_write_allowlist_limits_readwrite_targets` |
| `pg_roles` and privilege helper metadata | ✅ compatibility surface | `wire_spatial` privilege/catalog tests |
| pgwire TLS cert/key | ✅ configurable | ops docs and Kubernetes example |
| Metrics endpoint | ✅ opt-in, no SQL/secrets | metrics tests, read-only denial counter test, and external profile scrape |
| Read object/schema/table-level RBAC | ❌ not implemented | future trace-driven work |

## Trust boundaries

1. **Client → QuackGIS pgwire.** Use pgwire TLS or a trusted mTLS/proxy boundary
   before any non-dev deployment.
2. **QuackGIS → PostgreSQL catalog.** Catalog credentials are infrastructure
   secrets and should be scoped to the QuackGIS catalog database/schema.
3. **QuackGIS → object store.** Object credentials must be scoped to the dedicated
   prefix/bucket and rotated independently of pgwire users.
4. **Metrics endpoint.** Bind only on private interfaces or scrape through a
   trusted network; metrics never include SQL text, usernames, object paths, or
   secrets.
5. **Catalog and operational metadata.** `pg_catalog`, `information_schema`,
   DuckLake metadata UDTFs, snapshot history, and maintenance diagnostics can
   reveal object names or lifecycle state even when table reads are denied. Future
   object authorization must filter these surfaces consistently.
6. **Snapshot/maintenance operations.** Time travel, snapshot protection/restore,
   compaction, cleanup, and CDC have separate read/administrative effects and must
   not inherit permission from ordinary SELECT accidentally.

## Failure-mode probes before production claims

| Probe | Required behavior |
|---|---|
| Missing read/write password in password mode | server fails closed at startup |
| Missing TLS cert or key half | server fails closed at startup |
| Wrong password | connection denied; no fallback to trust mode |
| Read-only CREATE/COPY/DML/compaction | denied with SQLSTATE `42501` before catalog refresh or DuckLake mutation; DDL/DML/maintenance/unknown-call denials increment `quackgis_write_denied_total` |
| Write allowlist denial | non-allowlisted DuckLake `CREATE TABLE`/`COPY FROM STDIN`/DML/`ALTER TABLE`/compaction targets and indeterminate write statements are denied before planning; explicit-user `has_table_privilege`/`has_column_privilege` write metadata matches the allowlist |
| Secret rotation | rolling pods pick up new catalog/object/pgwire secrets; old credentials no longer work |
| TLS/mTLS enforcement | plaintext path is blocked by deployment/network policy when production profile requires TLS |
| Catalog/object credential revoke | in-flight operations fail explicitly; no partial data claim without mutation drill evidence |
| Unauthorized metadata/snapshot access | data, `pg_catalog`, metadata UDTFs, snapshot history, and maintenance diagnostics deny or filter the same object consistently |
| Unauthorized protection/restore/cleanup | denied before catalog/object mutation and recorded as a redacted administrative denial |
| Asset URI read | no signed URL, object credential, or unauthorized collection path is returned or logged |

External-service runs should combine this checklist with
[ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md).

## Object/schema/table RBAC target

Do not emulate PostgreSQL's full privilege system speculatively. Add object-level
authorization only when a real admin/client workflow requires it. The preferred
order:

1. deny-by-default write authorization remains at the DuckLake SQL hook; the
   read-only allowlist must classify new parser statement variants as denied until
   deliberately reviewed;
2. explicit service identities and schema/table allowlists for write-capable jobs
   are the first implemented object-level control (`QUACKGIS_WRITE_ALLOWLIST`);
3. read allowlists when a client/API deployment requires tenant separation;
4. `information_schema`/`pg_catalog`/DuckLake metadata filtered consistently with the data
   authorization decision;
5. separate administrative capabilities for compaction, snapshot protection,
   restore, retention cleanup, and future CDC; and
6. focused tests for every denied SQL shape: DDL, `COPY`, DML, compaction,
   metadata reads, asset URI discovery, snapshot/restore, cleanup, and CDC.

Authorization policy should be evaluated from normalized object identity, not raw
SQL text. A compatibility shortcut may shape a catalog response but may not bypass
the same object decision used by ordinary table access.

## Logging and audit posture

Current info logs record process-local query ids, protocol, pid, user, and
statement kind. Pgwire errors record only a bounded error class and user rather
than formatting the underlying error, which can contain SQL literals or storage
paths. These lines intentionally omit SQL text and object paths. If audit logs are
added later, they must be explicit opt-in, redacted by default, and covered by
tests that prevent secrets/object-store credentials from appearing.

A production audit event should have a stable schema: event id/time, authenticated
service identity, operation class, normalized schema/table or dataset id, outcome,
reason code, snapshot before/after where applicable, and correlation id. It should
record denied mutations, maintenance, protection/restore, retention cleanup, and
administrative policy changes without recording SQL literals, WKB, signed asset
URIs, passwords, tokens, or catalog/object credentials.

## Release claim rule

The compatibility matrix may claim production security only after evidence exists
for TLS/auth failure modes, secret rotation, external catalog/object credential
behavior, and any object-level RBAC surface that docs mention. Until then,
QuackGIS should be described as Alpha security/ops hardening with coarse roles.
