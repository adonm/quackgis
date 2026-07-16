# DuckDB roadmap alignment

This note turns upstream roadmap signals into conditional QuackGIS adoption
choices. It was reviewed on 2026-07-16 against:

- the official [DuckDB development roadmap](https://duckdb.org/roadmap), last
  updated June 2026;
- the official [DuckDB release calendar](https://duckdb.org/release_calendar),
  which tentatively lists 1.5.5 for 2026-07-20 and 2.0.0 for fall 2026;
- the official [DuckDB 1.5.4 release notes](https://duckdb.org/2026/06/17/announcing-duckdb-154.html),
  which identify 1.5.4 as non-LTS and include native geometry Parquet-statistics
  pruning plus `OPERATOR_ROW_GROUPS_SCANNED`;
- the official [Quack protocol](https://duckdb.org/quack/) beta documentation;
- the official [DuckLake roadmap](https://ducklake.select/roadmap), last updated
  April 2026; and
- the independent DuckDBLab/Olap Studio 1.5.4/v2 preview linked in the review
  request.

The official roadmap explicitly gives no delivery guarantee. The independent
article identifies itself as unaffiliated and labels its v2 feature list as
speculation. QuackGIS may align its boundaries and defer overlapping work based
on those signals, but it may claim or depend on a feature only after a released,
pinned artifact passes an executable gate.

As of that review, 1.5.5 and 2.0.0 remain future entries rather than released
artifacts. M5's required post-release evaluation and supported-bundle decision
therefore cannot close before those inputs exist; local package work may advance
without fabricating that release evidence.

## Decisions

| Upstream direction | QuackGIS burden it may remove | Alignment now | Adoption gate |
|---|---|---|---|
| DuckDB 2.0 and 1.5.5 | Carrying a short-lived engine baseline and patch-specific shims | Keep 1.5.4 as the current verified development bundle; evaluate 1.5.5 after release; make a supported post-1.5 bundle decision before Local 1.0 rather than publishing on a near-EOL line | Exact DuckDB/Spatial/DuckLake versions and digests; old-catalog reopen; write/reopen/backup/rollback; pgwire/catalog/client suites; 10M profiles; mixed-version refusal |
| PEG parser by default | Eventually, duplicate parser compatibility work and some error normalization | Keep the Rust PostgreSQL AST as the authorization boundary today. Add classic-versus-PEG acceptance/rejection equivalence to the engine upgrade matrix. Do not let runtime parser extensions widen pgwire SQL | Every maintained accepted query has equal result/type/plan behavior; every denied multi-statement, dynamic SQL, private catalog, authorization, and rewrite-bypass shape remains denied; error changes are classified |
| Stable Quack protocol | Potential future engine isolation if it can serve a complete attached-DuckLake data plane | Watch upstream only. The selected Local/Shared topology keeps ADBC and DuckDB in each complete worker and uses iroh at the client/control edge; Quack is not a competing deployment path | Explicit product-direction change plus a stable Rust client, complete DuckLake attach/data-plane support, streaming/backpressure, prepared parameters, transactions, cancellation, commit outcome, extension pinning, latency/RSS, crash/restart, and upgrade evidence |
| Async I/O | Blocking workers used only to hide synchronous native I/O, especially for object storage | Do not redesign around an internal core feature alone. If a supported client API exposes cancellable async operations, allow it to replace—not supplement—the corresponding blocking path | Same cancellation, memory, spill, first-row, throughput, and quarantine oracles; no unbounded task or native queue |
| C client/C extension API and Rust extensions | C++ internal-API/ABI maintenance for future custom extensions and possibly direct client FFI | Continue Arrow ADBC while it is the smaller proven boundary. Any new QuackGIS extension should prefer the supported C or Rust extension API once stable; do not port the temporary DuckLake fork merely to preserve it | Stable API and packaging, signed/versioned artifacts, vectorized workload benchmark, fuzz/property tests, upgrade matrix, and a smaller total support surface than SQL/Rust-edge alternatives |
| Continuous DuckLake improvement | Private metadata adapters, custom snapshot/version machinery, custom RBAC/storage policy, and bespoke summaries | Keep upstreaming `ducklake_column_info`; make official protected snapshots the intended M7 primitive; evaluate DuckLake RBAC as a storage-enforcement backend; wait for official UDT/materialized-view primitives before inventing equivalents | Public versioned API; transactional and independent-reader evidence; PostgreSQL-facing semantic mapping; backup/restore/upgrade; no second writer or schema authority |
| Partition/sort/sample/profile improvements | Additional bbox/index/statistics side structures and custom scan diagnostics | Freeze expansion of custom pruning beyond measured release needs. Re-run M4 against each engine candidate and delete bbox machinery when native plans meet exactness and scan-byte budgets | Exact-oracle matrix, visible exact recheck, scan bytes/row groups, 10M then 100M profiles |

The native-geometry comparison is immediate, not only a v2 watch item: the pinned
1.5.4 release already claims repaired geometry Parquet pruning and exposes a row-
group counter. M4 must use that counter and a native `GEOMETRY` baseline before
adding more WKB bbox machinery.

## Speculative v2 features

The independent article speculates about stronger streaming, geospatial support,
concurrent transactions, extension tooling, and performance. QuackGIS will be
aggressive about adopting those capabilities when released because they could
remove code:

- native spatial functions/statistics may delete compatibility macros, bbox
  maintenance, and candidate injection;
- a supported streaming/async client may delete portions of the blocking worker
  and cancellation adapter;
- stronger transaction or public metadata hooks may collapse post-commit catalog
  reconciliation into one native transaction;
- stable Rust/C extensions may make the five measured extension candidates
  cheaper than C++ forks; and
- a future remote engine protocol is relevant only if it can replace the complete
  worker's engine boundary without creating a second Local/Shared topology.

None of those speculative capabilities changes a current claim. In particular,
“improved concurrent transactions” does not remove QuackGIS catalog serialization
unless a released API can atomically expose and map uncommitted durable DuckLake
identity.

## DuckLake-specific adoption

The DuckLake roadmap is more directly relevant than generic v2 speculation:

- **Protected snapshots:** adopt rather than implement a competing protection
  mechanism; M7 remains blocked until the official primitive and recovery gates
  exist.
- **RBAC:** potentially use it as the storage enforcement backend, while the Rust
  edge still owns PostgreSQL `session_user`/`current_user`, role assumption,
  catalog projection, SQLSTATEs, and HTTP role mapping.
- **User-defined types:** re-evaluate native geometry persistence and
  subtype/SRID/dimension identity before expanding WKB conventions.
- **Materialized views/incremental maintenance:** prefer them for maintained
  extent/tile summaries instead of a QuackGIS refresh engine.
- **Primary-key syntax without enforcement:** do not expose PostgreSQL constraint
  or uniqueness claims from syntax alone. Metadata-only declarations cannot
  replace enforcement evidence.
- **Metadata-scan/Bloom-filter improvements:** measure them before adding another
  locality or side-index mechanism.

## Explicit non-adoptions

- Do not expose Quack directly to PostgreSQL/GIS clients; QuackGIS remains the
  PostgreSQL compatibility and authorization edge.
- Do not enable runtime-extensible grammar from client input.
- Do not add a second engine transport or Quack deployment profile without an
  explicit product-direction change and complete attached-DuckLake evidence.
- Do not wait for future DuckLake RBAC to implement the PostgreSQL-facing role
  contract needed by Local 1.0.
- Do not treat `MATCH_RECOGNIZE`, parallel Python UDFs, installers, or C++17 as
  release requirements.
- No maintained SQL uses the deprecated `x -> ...` lambda syntax; continue to
  reject introducing it before DuckDB 2.0 removes it by default.
