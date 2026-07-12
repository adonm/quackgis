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
| TLS/SCRAM/startup | Rust edge | fail-closed material checks, explicit TLS-required startup policy, real SCRAM workflow | encrypted client, plaintext-denial, and restart-rotation evidence |
| parsed read/write policy | Rust edge | unit + denied real-client cases | filtered metadata/admin permissions |
| portals/fetch paging | Rust edge | live ADBC stream plus three-page native workflow; native partial-drop quarantine proof | realistic fetch sizes and memory profile |
| query cancellation | native + Rust edge | `57014`, explicit quarantine/fresh reuse, and clean 100-cancel reference at 1.51 ms p95 with zero native failures | write/commit cancellation disposition |
| query admission/resources | Rust edge + native settings | bounded connection/active/queued queries; fixed native worker pool with reserved control slot; queue timeout; DuckDB threads/memory/temp/spill config; 32-client/eight-reader suspended-portal proof plus all-class queue/completion profile | write/commit interruption and mixed-workload soak |
| Arrow result encoding | Rust edge | one ADBC batch at a time; configured ceiling/metrics; clean 1M/10M generated-BIGINT reference RSS/first-row profiles | wider variable-width/native-batch RSS and type fuzzing |
| COPY FROM STDIN | Rust edge + native ingest | incremental bounded batches; >20 MiB/220k-row stream; atomic malformed/cancel/disconnect/timeout behavior; scalar/NULL/WKB reopen; compaction | idle-wait error delivery and 10M/1 GiB RSS/throughput gates |
| transactions/session isolation | native + Rust ownership | commit/rollback/disconnect workflow | timeout/cancel/uncertain cleanup and pool reuse |
| local official DuckLake | native | create/ingest/query/snapshot/merge/reopen | backup/restore/upgrade/soak |
| shared DuckLake | blocked | startup fails closed | after Local 1.0: official managed profile evidence |
| storage authority | Rust edge | local marker tests | shared credentials/authority design |
| WKB/EWKB transport | native + Rust encoding | exact WKB ingest/query/reopen | EWKB/SRID/client matrix |
| 31 native spatial cases | native | real pgwire corpus | client-driven expansion only |
| 5 spatial aliases | macro/rewrite | real pgwire corpus | delete when native contract matches |
| 6 compatibility macros | macro/rewrite | real pgwire corpus | NULL/empty/overload/property coverage |
| 10 spatial/catalog gaps | blocked/Rust edge | classified ledger + stable simple/extended `0A000` | prioritize only release-client requirements |
| `ST_NDims`/`ST_CoordDim`/`ST_GeometryN` | extension candidate | classified + stable simple/extended `0A000` | proposal requires workload + vector benchmark |
| exact bbox recheck | native query | small storage oracle | safe injection plus holes/invalid/scale plans |
| layout/locality maintenance | partial native SQL | COPY computes four reserved bbox columns; spatial/reserved DML fails closed; ordinary-column bound UPDATE preserves WKB/bbox through reopen | safe AST predicate injection, geometry mutation maintenance, compaction/layout scale evidence |
| PostgreSQL catalogs | blocked/Rust edge | client-neutral fixture for DuckDB-derived table/column metadata and ordinary native catalog behavior; broad metadata denied | captured psql/psycopg/OGR/QGIS surfaces |
| geometry OID discovery | partial Rust edge | client-neutral structural sentinel lookup + RowDescription/text/binary/NULL pgwire fixture | named QGIS/OGR discovery and subtype/SRID/dimension identity |
| psql/psycopg | partial | tokio-postgres is maintained test client | version-pinned named workflows |
| GDAL/OGR | blocked | prior traces only | read + streaming COPY copied-data test |
| QGIS read-only | blocked | prior traces only | discovery/filter/identify/render test |
| GeoServer/Martin/editing/BI | deferred | historical oracles only | reconsider after Local 1.0 surface is stable |
| runtime packaging | native artifacts + Rust | static verified image contract | clean-room image run, upgrade matrix |
| query/ingest observability | partial | process/auth/admission/cancel counters, COPY rows/bytes/batches/duration/commit latency, sampled native memory/spill | profile evidence |
| health/readiness | Rust edge + native probe | process liveness separated from pgwire-bind and read-only DuckLake snapshot readiness; drain/failure states | write-capacity SLO and remote dependency probes |
| backup/restore/upgrade | blocked | restart/reopen only | Local 1.0 operational gates |

## Maintenance rule

Each supported row must name an executable gate. New compatibility requirements
must start from a maintained client/workload and follow the decision ladder in
`PROJECT_DIRECTION.md`. When DuckDB satisfies a contract natively, delete the
QuackGIS workaround and update this table in the same change.
