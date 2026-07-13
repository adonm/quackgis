# Engine capability ledger

This ledger assigns each requirement to its DuckDB-first implementation level and
current evidence. It is not a list of behavior inherited from retired engines.

Disposition:

- **native** — DuckDB/official extension owns execution;
- **macro/rewrite** — optimizer-visible SQL compatibility;
- **Rust edge** — PostgreSQL protocol/catalog/control behavior;
- **extension candidate** — measured vectorized gap requiring a proposal;
- **blocked** — required but not implemented;
- **deferred** — outside the current release.

| Capability | Owner/disposition | Current evidence | Next gate |
|---|---|---|---|
| simple/extended pgwire | Rust edge | native workflow; maintained SET/SHOW; AST `public` mapping; quoted COPY targets | wider parameters and catalog-backed client discovery |
| TLS/SCRAM/startup | Rust edge | actual-process encrypted client, hostname/trust verification, SCRAM, plaintext denial, and restart-based certificate/password rotation profile | packaged Kind rotation and production failure drill |
| parsed read/write policy | Rust edge | unit + denied real-client cases | common role/membership/grant engine shared by privilege inquiry, catalogs, execution, and OpenAPI |
| portals/fetch paging | Rust edge | live ADBC stream plus three-page native workflow; native partial-drop quarantine proof | realistic fetch sizes and memory profile |
| query/write cancellation | native + Rust edge | query `57014`, explicit quarantine/fresh reuse, clean 100-cancel reference at 1.51 ms p95, autocommit-write rollback/reuse, and explicit-transaction write rollback/quarantine | commit is non-cancellable; response-loss reconciliation remains operational work |
| query admission/resources | Rust edge + native settings | bounded connection/active/queued queries; fixed native worker pool with reserved control slot; queue timeout; DuckDB threads/memory/temp/spill config; 32-client/eight-reader suspended-portal proof plus all-class queue/completion profile | write/commit interruption and mixed-workload soak |
| Arrow result encoding | Rust edge | one ADBC batch at a time; configured ceiling/metrics; clean 1M/10M BIGINT and 1M nullable VARCHAR/BLOB reference RSS/exact-value profiles | maximum driver-batch/additional type shapes and type fuzzing |
| COPY FROM STDIN | Rust edge + native ingest | pre-body frontend-frame ceiling; incremental bounded chunks/rows/Arrow batches; >20 MiB/220k-row stream; atomic malformed/cancel/disconnect/timeout behavior; scalar/NULL/WKB reopen; compaction; clean 10M reference at 126 MiB RSS delta and 0.528 pgwire/direct ratio | dependency-limited idle-wait error delivery remains documented |
| transactions/session isolation | native + Rust ownership | commit/rollback/disconnect workflow; failed `25P02`; cancellable pre-commit writes; non-cancellable commit with indeterminate-failure classification | commit response-loss reconciliation and soak |
| local official DuckLake | native | create/ingest/query/snapshot/merge/reopen plus checksummed offline exact-path backup/restore | online/relocated recovery, upgrade, and soak |
| shared DuckLake | blocked | startup fails closed | after Local 1.0: official managed profile evidence |
| storage authority | Rust edge | local marker tests | shared credentials/authority design |
| WKB/EWKB transport | native + Rust encoding | exact WKB ingest/query/reopen | EWKB/SRID/client matrix |
| 31 native spatial cases | native | real pgwire corpus | client-driven expansion only |
| 5 spatial aliases | macro/rewrite | real pgwire corpus | delete when native contract matches |
| 6 compatibility macros | macro/rewrite | real pgwire corpus | NULL/empty/overload/property coverage |
| 10 spatial/catalog gaps | blocked/Rust edge | classified ledger + stable simple/extended `0A000` | prioritize only release-client requirements |
| `ST_NDims`/`ST_CoordDim`/`ST_GeometryN` | extension candidate | classified + stable simple/extended `0A000` | proposal requires workload + vector benchmark |
| exact bbox recheck | native query | small storage oracle | safe injection plus holes/invalid/scale plans |
| layout/locality maintenance | partial native SQL | COPY computes four reserved bbox columns; numbered-bound/NULL geometry UPDATE atomically refreshes them with malformed/rollback/reopen evidence; direct INSERT, arbitrary geometry expressions, and reserved writes fail closed | broader geometry mutation policy, safe AST predicate injection, compaction/layout scale evidence |
| PostgreSQL catalog snapshot/epoch | blocked/Rust edge | `pg18-column-core-v1`; DuckDB 1.5.4 multi-process gate proves durable DuckLake table UUID/ID and column field IDs; broad metadata denied | public authoritative schema/column identity extraction, transactional PostgreSQL OID/attribute registry, immutable snapshot, commit/rollback invalidation |
| core `pg_catalog`/`information_schema` | partial/Rust edge + native relational evaluation | structural explicit-reference mapping to protected process-local namespace/type/range/owner-role views; provenance-bound PostgreSQL wire hints; typed custom lookup and ordinary rows; alias-safety regression | built-in types, class/attribute/database surfaces, `reg*`, implicit catalog lookup, RowDescription origins, privilege-aware discovery |
| roles/session/request context | blocked/Rust edge | startup SCRAM identities and maintained SET/SHOW only | configuration-backed LOGIN/NOLOGIN roles, memberships, current/session user, SET/SET LOCAL/RESET ROLE, bounded transaction-local claims |
| PostgreSQL object privileges | blocked/Rust edge | read/write/maintenance allowlists | ownership/grants, `pg_has_role`, `has_*_privilege`, common enforcement, non-widening allowlist migration |
| PostgreSQL RLS | deferred/Rust edge + native exact predicates | none; no RLS claim | separate policy model, catalog consistency, structural injection, adversarial read/write bypass suite |
| geometry/geography OID discovery | partial Rust edge | relational namespace/type/range rows; structural catalog mapping; seven PostgreSQL 18 result types; ordinary/unknown-OID queries; RowDescription text/binary/NULL pgwire fixture | named QGIS/OGR discovery and subtype/SRID/dimension identity |
| psql/psycopg | partial | tokio-postgres maintained client; psql 18.3 TLS/SCRAM scalar Kind smoke; exact psql 18.3 `\d+` oracle freezes 12 PostgreSQL 18.4 catalog query families and rendered spatial table structure | implement traced relation/attribute/index/constraint surfaces; copied-data psql and psycopg workflows |
| GDAL/OGR | partial | pinned 3.11.5 TLS/SCRAM scalar Kind smoke plus digest-pinned PostgreSQL 18.4/PostGIS copied-point oracle with 21 normalized discovery query families; QuackGIS discovery still reaches unsupported `ST_SRID` | implement traced class/attribute/index/description/spatial metadata, read + streaming COPY copied-data test |
| QGIS read-only | blocked | prior traces only | discovery/filter/identify/render test |
| role-aware REST/OpenAPI | blocked/stateless pgwire client | authenticated read-only bearer preview with independent schema cache | JWT role mapping, authenticator role, catalog/privilege discovery, role+epoch cache, packaged replicas |
| GeoServer/Martin/editing/BI | deferred | historical oracles only | reconsider after Local 1.0 surface is stable |
| runtime packaging | native artifacts + Rust | static verified image contract | clean-room image run, upgrade matrix |
| query/ingest observability | partial | process/auth/admission/cancel counters, COPY rows/bytes/batches/duration/commit latency, sampled native memory/spill | profile evidence |
| health/readiness | Rust edge + native probe | process liveness separated from pgwire-bind and read-only DuckLake snapshot readiness; drain/failure states | write-capacity SLO and remote dependency probes |
| backup/restore/upgrade | partial | checksummed offline exact-path backup/restore plus restart/reopen | online/relocated production recovery, upgrade/rollback, and release-catalog timing |

## Maintenance rule

Each supported row must name an executable gate. New compatibility requirements
must start from a maintained client/workload and follow the decision ladder in
`PROJECT_DIRECTION.md`. When DuckDB satisfies a contract natively, delete the
QuackGIS workaround and update this table in the same change.
