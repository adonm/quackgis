# Security and RBAC hardening plan

QuackGIS' current security surface is intentionally small: pgwire SCRAM password
auth, optional pgwire TLS, a configured read/write user, an optional read-only
user, and fail-closed write authorization at the DuckLake SQL boundary. Full
PostgreSQL RBAC is not implemented.

This document defines what must be true before widening the security claim.

## Current implemented controls

| Control | Status | Evidence |
|---|---|---|
| Trust-mode local development | ✅ default preview | local smokes |
| SCRAM-SHA-256 password auth | ✅ implemented | `password_auth_and_readonly_role_fail_closed` |
| Read/write vs read-only roles | ✅ coarse role model | read-only writes fail closed; recognized DDL/DML/maintenance denials increment `quackgis_write_denied_total` in wire tests |
| `pg_roles` and privilege helper metadata | ✅ compatibility surface | `wire_spatial` privilege/catalog tests |
| pgwire TLS cert/key | ✅ configurable | ops docs and Kubernetes example |
| Metrics endpoint | ✅ opt-in, no SQL/secrets | metrics tests, read-only denial counter test, and external profile scrape |
| Object/schema/table-level RBAC | ❌ not implemented | future trace-driven work |

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

## Failure-mode probes before production claims

| Probe | Required behavior |
|---|---|
| Missing read/write password in password mode | server fails closed at startup |
| Missing TLS cert or key half | server fails closed at startup |
| Wrong password | connection denied; no fallback to trust mode |
| Read-only CREATE/COPY/DML/compaction | denied before DuckLake mutation; recognized DDL/DML/maintenance denials increment `quackgis_write_denied_total` |
| Secret rotation | rolling pods pick up new catalog/object/pgwire secrets; old credentials no longer work |
| TLS/mTLS enforcement | plaintext path is blocked by deployment/network policy when production profile requires TLS |
| Catalog/object credential revoke | in-flight operations fail explicitly; no partial data claim without mutation drill evidence |

External-service runs should combine this checklist with
[ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md).

## Object/schema/table RBAC target

Do not emulate PostgreSQL's full privilege system speculatively. Add object-level
authorization only when a real admin/client workflow requires it. The preferred
order:

1. deny-by-default write authorization remains at the DuckLake SQL hook;
2. schema/table allowlists for write-capable service accounts;
3. read allowlists only if a client/API deployment requires tenant separation;
4. `information_schema`/`pg_catalog` metadata filtered consistently with the data
   authorization decision;
5. focused tests for every denied SQL shape: DDL, `COPY`, DML, compaction, and
   future snapshot/CDC operations.

## Logging and audit posture

Current info logs record process-local query ids, protocol, pid, user, and
statement kind. They intentionally omit SQL text and object paths. If audit logs
are added later, they must be explicit opt-in, redacted by default, and covered by
tests that prevent secrets/object-store credentials from appearing.

## Release claim rule

The compatibility matrix may claim production security only after evidence exists
for TLS/auth failure modes, secret rotation, external catalog/object credential
behavior, and any object-level RBAC surface that docs mention. Until then,
QuackGIS should be described as Alpha security/ops hardening with coarse roles.
