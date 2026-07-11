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
| simple/extended pgwire | Rust edge | native pgwire workflow | streaming results, deadlines, wider parameters |
| TLS/SCRAM/startup | Rust edge | unit + real SCRAM workflow | TLS-required mode, rotation/client evidence |
| parsed read/write policy | Rust edge | unit + denied real-client cases | filtered metadata/admin permissions |
| portals/fetch paging | Rust edge | three-page native workflow | same live ADBC stream, realistic fetch sizes |
| query cancellation | blocked | protocol cancel handler only | cancel active ADBC statement, latency/quarantine test |
| query admission/resources | blocked | no active execution/admission bound | connection/reader/writer/worker/memory/spill budgets |
| Arrow result encoding | Rust edge | scalar and `arrow-pg` tests | stream batches; fuzz every advertised type |
| COPY FROM STDIN | Rust edge + native ingest | bounded scalar/WKB workflow | incremental batches, escaping/types, 1 GiB gate |
| transactions/session isolation | native + Rust ownership | commit/rollback/disconnect workflow | timeout/cancel/uncertain cleanup and pool reuse |
| local official DuckLake | native | create/ingest/query/snapshot/merge/reopen | backup/restore/upgrade/soak |
| shared DuckLake | blocked | startup fails closed | after Local 1.0: official managed profile evidence |
| storage authority | Rust edge | local marker tests | shared credentials/authority design |
| WKB/EWKB transport | native + Rust encoding | exact WKB ingest/query/reopen | EWKB/SRID/client matrix |
| 31 native spatial cases | native | real pgwire corpus | client-driven expansion only |
| 5 spatial aliases | macro/rewrite | real pgwire corpus | delete when native contract matches |
| 6 compatibility macros | macro/rewrite | real pgwire corpus | NULL/empty/overload/property coverage |
| 10 spatial/catalog gaps | blocked/Rust edge | classified ledger | prioritize only release-client requirements |
| `ST_NDims`/`ST_CoordDim`/`ST_GeometryN` | extension candidate | classified only | proposal requires workload + vector benchmark |
| exact bbox recheck | native query | small storage oracle | safe injection plus holes/invalid/scale plans |
| layout/locality maintenance | blocked | explicit fixture columns only | DuckDB COPY/compaction implementation |
| PostgreSQL catalogs | blocked/Rust edge | broad metadata denied | captured psql/psycopg/OGR/QGIS surfaces |
| geometry OID discovery | partial Rust edge | encoder sentinel tests | `pg_type`/RowDescription in named clients |
| psql/psycopg | partial | tokio-postgres is maintained test client | version-pinned named workflows |
| GDAL/OGR | blocked | prior traces only | read + streaming COPY copied-data test |
| QGIS read-only | blocked | prior traces only | discovery/filter/identify/render test |
| GeoServer/Martin/editing/BI | deferred | historical oracles only | reconsider after Local 1.0 surface is stable |
| runtime packaging | native artifacts + Rust | static verified image contract | clean-room image run, upgrade matrix |
| query/ingest observability | blocked | process/auth counters | M1/M2 resource and performance metrics |
| backup/restore/upgrade | blocked | restart/reopen only | Local 1.0 operational gates |

## Maintenance rule

Each supported row must name an executable gate. New compatibility requirements
must start from a maintained client/workload and follow the decision ladder in
`PROJECT_DIRECTION.md`. When DuckDB satisfies a contract natively, delete the
QuackGIS workaround and update this table in the same change.
