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
| TLS/SCRAM/startup | Rust edge | actual-process encrypted client, hostname/trust verification, SCRAM, plaintext denial, and restart-based certificate/password rotation; packaged Kind mTLS ingress plus edge-key rotation denies plaintext, missing/old certificates, and direct worker TCP | JWT/database rotation and production revocation/failure drills |
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
| exact bbox recheck | native query + Rust AST edge | one-table maintained-layout `ST_Intersects` injects four conservative candidates while retaining the exact predicate; unit edge cases and pgwire exact oracles pass. Two clean 10M mixed point/line/polygon references compare maintained WKB/bbox and native `GEOMETRY`, with equal exact counts, at least 20x row-group reduction, conservative compressed scan bytes below 5%, 15.46–20.52 ms pgwire p95, at most 1.90 MiB process-RSS growth, 10.10 MiB DuckDB-memory growth, and zero spill | grouped aggregate/join/wide-projection workloads and two 100M references |
| layout/locality maintenance | partial native SQL | COPY computes four reserved bbox columns; numbered-bound/NULL geometry UPDATE atomically refreshes them with malformed/rollback/reopen evidence; direct INSERT, arbitrary geometry expressions, and reserved writes fail closed. The mixed-shape profile compacts both 10M layouts from 25 files to one without changing exact results. Local 1.0 retains maintained WKB/bbox storage; native files are about 45% smaller but lack the required write/client contract | broader geometry mutation/compaction policy and native-representation deletion gate |
| PostgreSQL catalog snapshot/epoch | active/Rust edge | `pg18-column-core-v1`; checksum-pinned DuckLake 1.5.4 development lane allocates durable namespace/relation/row-type OIDs and attribute numbers, retains tombstones, serializes commit/reconcile pairs, validates registry/snapshot invariants, advances a schema/default/comment fingerprint epoch, fails guarded catalog reads in the reconciliation gap, invalidates prepared reads on epoch change, and emits matching direct-column RowDescription origins | obtain durable empty-schema identity; consume epochs in REST/other caches; broaden safe expression provenance; upstream acceptance and signed official bundle remain release gates |
| core `pg_catalog`/`information_schema` | C3 complete; C5 discovery active in Rust/native lane | baseline namespace/database/type/range/collation/owner-role views plus development-only registry-backed object/default/comment/NOT-NULL catalogs; `pg_index` is explicitly empty; role-bound information schema advertises generic `geometry_columns` and typed-empty `spatial_ref_sys`; registered objects, casts, and `format_type` remain fail-closed | signed upstream identity bundle; authoritative CRS/subtype/dimension metadata and richer provenance; upstream primary/unique/foreign-key/index support or an explicit client limitation policy |
| roles/session/request context | C4 complete/Rust edge | bounded immutable role/owner/grant config; cycle/LOGIN validation; name-typed session/effective identity; original-login set-option assumption; SET/SET LOCAL/NONE/RESET; transaction cleanup and prepared invalidation; exact 16 KiB transaction-local `request.jwt.claims` set/get with commit/failed-rollback plus cancellation/quarantine/fresh-session isolation through actual pgwire | role-aware OpenAPI/JWT visibility |
| PostgreSQL object privileges | active/Rust edge | common schema-USAGE/table owner/direct/inherited/PUBLIC decision gates SELECT, DML, COPY/INSERT, MAINTAIN, and predeclared-owner CREATE; relational role/membership/owner catalogs, bounded PostgreSQL 18 privilege inquiry, and role-aware schema/table/column/grant information schema consume the same decisions; actual pgwire proves inquiry/discovery/read/write agreement; legacy allowlists remain an outer ceiling | OpenAPI consistency; broader column grants remain deferred |
| PostgreSQL RLS | deferred/Rust edge + native exact predicates | none; no RLS claim | separate policy model, catalog consistency, structural injection, adversarial read/write bypass suite |
| geometry/geography OID discovery | partial Rust edge | relational namespace/type/range rows; structural catalog mapping; seven PostgreSQL 18 result types; ordinary/unknown-OID queries; RowDescription text/binary/NULL pgwire fixture | named QGIS/OGR discovery and subtype/SRID/dimension identity |
| psql/psycopg | partial | tokio-postgres maintained client; psql 18.3 TLS/SCRAM scalar Kind smoke; traced default/comment/NOT-NULL surfaces and empty index identity now execute, while the exact psql 18.3 `\d+` oracle freezes the remaining query families | widen safe psql query shapes; copied-data psql and psycopg workflows; document absent key/index semantics |
| GDAL/OGR | partial | pinned 3.11.5 TLS/SCRAM scalar Kind smoke plus digest-pinned PostgreSQL 18.4/PostGIS copied-point oracle; traced generic geometry metadata, empty-geometry SRID, typed-empty CRS, and extent shapes pass focused actual pgwire, while DuckLake exposes no primary key | read + streaming COPY copied-data test, explicit no-FID behavior, and authoritative CRS metadata policy |
| QGIS read-only | blocked on end-to-end qualification | digest-pinned offscreen QGIS 3.44.11 oracle freezes 32 statements/26 families; session bootstrap and bounded generic spatial metadata/version/extent surfaces pass focused actual pgwire | run open/fields/CRS/count/extent/binary identify against QuackGIS; resolve generic SRID 0/empty CRS behavior; add filter/render |
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
