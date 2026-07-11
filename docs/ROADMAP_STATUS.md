# Roadmap status index

This index keeps implemented local contracts out of the forward roadmap. A doc or
runbook listed here is a useful contract, but the matching roadmap item stays open
until the intended evidence has run at its target scale and source SHA.

The active execution loop keeps the local Kind+Linkerd envelope in
[LOCAL_KIND_LINKERD_FOCUS.md](./LOCAL_KIND_LINKERD_FOCUS.md) as a cheap companion
gate while promoting the same claims to managed external services. Local
refinement must not postpone the first external evidence run indefinitely.

## Closed contracts and active frontiers

| Area | Implemented contract / evidence floor | Active frontier |
|---|---|---|
| Trend dashboards and release evidence | `just metrics-dashboard`; `just metrics-budget-check`; scheduled workflows upload dashboards; `docs/RELEASE_EVIDENCE.md` defines release packet | attach selected scheduled/manual artifacts to real releases with required budget assertions |
| Local Kind+Linkerd Alpha | `docs/LOCAL_KIND_LINKERD_FOCUS.md` defines the maximum local execution ladder, scale knobs, and claim boundaries | make full-ladder artifacts routine and budgeted |
| External PostgreSQL/S3 Alpha | `docs/ALPHA_EXTERNAL_SERVICES.md` defines credential rotation, restart, throttling, backup/restore, cleanup, refresh, and PostgreSQL catalog interoperability-result drills; `just external-alpha-evidence-check` validates redacted packet manifests against collected metrics, requires restore RPO/RTO and failed-writer quarantine fields for passed drills, and requires an explicit standard/non-standard catalog interop result plus migration implication so wiring smokes cannot be mislabeled as Alpha promotion | run against real platform-managed PostgreSQL/S3 services and publish artifacts |
| Benchmark ladder | `docs/ANALYTICS_BENCHMARKS.md`; manual `Benchmark ladder` workflow; QPS/OLAP/compaction metrics; validated `layoutbench-regional-r100m-v1` defines exactly 100M rows and 202 load batches; PostgreSQL metadata-provider calls are process-metered; snapshot-fresh extended execution has a 7-call local oracle; the bounded Kind runner seeds/measures the exact profile and the profile-bound parser enforces warm/cold/direct/refresh budgets | execute the 100M, billion-row, real-data, managed-service, and release-budget ladders; physical read/write roundtrip instrumentation remains open |
| Real-data client matrix | `docs/REAL_DATA_CLIENT_MATRIX.md`; OSM Monaco opt-in baseline includes OGR copy/read, SQL sample parity, MVT SQL bytes with real `name` attribute tokens, QGIS open/filter/render, and metrics-report extraction for row/GeoJSON/QGIS/MVT counts; MVT encoder/SQL probes have key/value tags and the real Martin binary opt-in proves configured attributes on a synthetic layer | add GeoServer and real Martin binary/OSM attribute side-by-side, then wider OSM/Overture-derived layers |
| API/client expansion | `docs/API_CLIENT_PROBES.md`; `just api-client-local-smoke` is in `just ci`; `just kind-api-client-probe` is in `just kind-compatibility` and compatibility metrics for pgwire/catalog/WKB/bbox/BI/MVT profile surfaces, including MVT attribute tags | promote to named psycopg, SQLAlchemy/GeoPandas, pg_featureserv, MVT, and BI client probes over real dependencies/data |
| PostGIS conformance | `docs/POSTGIS_CONFORMANCE.md`; `just postgis-regress`; `just postgis-conformance-summary` | promote broader fixture families through pgwire/client traces |
| DuckDB spatial migration | `just duckdb-spatial-compat-probe` machine-classifies all 57 maintained cases, and the real-driver pgwire workflow executes all 40 native/rewrite/macro cases through the feature-gated DuckDB server with maintained scalar results; `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` owns the 31 native / 5 rewrite / 4 macro / 12 Rust-edge / 5 extension-candidate split | implement the five extension candidates, route the 12 Rust-edge/catalog cases, then expand classification to SQL portability and maintained client traces |
| DuckDB runtime packaging | A separate digest-pinned Linux x86_64 image context verifies committed `libduckdb`/`ducklake`/`spatial` SHA-256 values, records CLI/server/source digests, forbids online install steps, runs non-root, and passes `LOAD spatial; LOAD ducklake` with container networking disabled; CI artifacts runs the build/smoke. The in-process ADBC boundary and feature-gated server startup independently require the exact library digest and SQL runtime version before claiming a data root. | finish pgwire/client parity, publish architecture/upgrade matrices, verify extension digests independently of the DuckDB loader, and run clean-room/rolling/soak/managed-service gates |
| DuckDB pgwire routing | The bounded local D2 workflow now passes through the real feature-gated `--engine-backend=duckdb` CLI route: structural single-statement SELECT/CREATE/INSERT/UPDATE/DELETE routing, parameterized reads and mutations, text COPY→Arrow for boolean/numeric/date/timestamp/text/WKB core types, exact spatial SELECT, per-client transactions/disconnect rollback, SCRAM plus normalized read/write allowlists before ADBC prepare, denial audit/metrics, official snapshot inspection, restart/reopen, paged portals, empty-result metadata, and maintained scalar RowDescription/encoding. Encoded rows flow directly from materialized Arrow batches instead of collecting a second row buffer. Required CI runs the real-driver gate. | add native cancellation and ADBC batch/COPY backpressure, COPY options/escaping and remaining types, catalog/PostGIS compatibility, shared storage, and maintained clients before default-backend promotion |
| Native mutation safety | local native DML/compaction tests; before-commit failpoint retry oracles and abort metric; six real-process `SIGKILL` cases cover delete/update/explicit bucket compaction before and after commit, prove exact prewrite-to-inventory equality before commit, committed-path exclusion after commit, restart state, and explicit before-commit retry; offline `--orphan-inventory` remains age-gated and dry-run by default, while explicit `--orphan-quarantine-prefix --orphan-quarantine-apply` copies candidates outside live data and rechecks before source removal; `docs/MUTATION_FAILURE_DRILLS.md` | extend the process/quarantine matrix to Kind/managed-service storage, transaction batching, permanent deletion proof, response-loss reconciliation, and reference-reader interop |
| DuckDB/DuckLake migration | The current SQLite/local and PostgreSQL multicatalog writers remain the default and are not official-reader compatible; the first DuckDB attach exposed missing allocator metadata. The feature-gated ADBC kernel and local CLI backend pass a real-libduckdb official-DuckLake slice for Arrow/COPY ingestion, exact spatial query/reopen, transactions, structural pgwire routing, SCRAM/table policy, fail-fast reentrant access, and unsafe-connection quarantine. `mise.toml`/`mise.lock` pin DuckDB CLI 1.5.4, while an explicit Linux x86_64 bootstrap verifies official libduckdb checksums, preinstalls signed `ducklake`/`spatial` extensions into an isolated local home, records their digests, and lets ADBC plus independent CLI engine/authority gates run with `LOAD` only; those real native gates run in required pull-request CI. A DataFusion-free `engine_api` owns Arrow statement/query/ingest, bound mutation, schema, snapshot, maintenance, and classified-error types. One process-owned database opens independent session connections, and write-capable roots atomically record one storage authority. `docs/ENGINE_CAPABILITY_LEDGER.md` owns D0 status. | migrate legacy pgwire onto streaming/per-session/cancellation contracts, then execute legacy export/import, full spatial/client parity, shared operations, production platform packaging, cutover, and old-engine retirement |
| Spatial family identity | Explicit SQL geometry/geography persist as Binary WKB/EWKB plus validated Arrow metadata and snapshot-versioned DuckLake `column_type`; dynamic metadata tables, RowDescription, `pg_attribute`, and pgjdbc are metadata-first; an unconventional-name UPDATE/compaction/restart pgwire test proves OIDs and no `geom TEXT` false positive; old conventional Binary remains compatible | durable subtype/SRID/dimensions, old-blob migration, geography reference-reader interoperability, generic pg_type/typmod fidelity, and PostgreSQL/S3 external evidence |
| Spatial layout | ordinary hidden bbox/time/space/Morton columns, statistics-based pruning, exact recheck, and native bucket compaction are implemented; a real DuckDB/official-DuckLake oracle now proves conservative bbox over-selection, exact `ST_Intersects` recheck retained in the plan, and reopen equality | DuckDB time/Morton maintenance, real-data scale, structural partition/file stats, catalog budgets, and only then possible true coarse DuckLake partitions |
| Pgwire protocol boundary | raw pre-parse rewrites, parsed hooks, type/catalog encoding, dedicated COPY, maintained cursor paths, and raw extended `Execute.max_rows` paging are covered; the legacy oracle proves suspension/resume/final completion, and the DuckDB backend repeats three ordered `max_rows=1` portal pages. DuckDB statement routing now uses the PostgreSQL AST rather than SQL prefixes. | run realistic pgjdbc fetch-size evidence at city scale, add native ADBC query streaming/cancellation, and structurally replace remaining legacy/COPY SQL-text classifiers |
| Snapshot/time travel | `docs/SNAPSHOT_OPERATIONS.md`; literal snapshot-id and RFC3339 timestamp `AS OF` plus named selectors, exact-id validation, deterministic timestamp resolution, count/extent parity, metadata UDTFs, no-op catalog reopen, counters, and a matched-backup local rollback oracle that validates a prior head after source advancement | run rollback against managed services; implement protected retention, live release switching, and safe CDC row UDTFs |
| Security/RBAC | SCRAM/read-only/TLS docs and tests; configured TLS fails startup on invalid certificate/key material; structural read/write policy returns SQLSTATE `42501`; normalized write/read allowlists cover data and unfiltered metadata surfaces; bounded error logs, denial metrics, and redacted audit events are covered. The real DuckDB CLI process now repeats SCRAM writer/reader startup and proves allowed reads plus denied INSERT, COPY, non-allowlisted reads, and DuckLake metadata before ADBC prepare. `docs/SECURITY_RBAC.md` defines the RBAC target. | execute external secret/TLS failure drills, replace fail-closed metadata denial with trace-driven filtered metadata views where required, expand audit coverage to snapshot/restore/retention/admin changes, and split future administrative permissions |
| Multi-modal assets | `docs/MULTIMODAL_ASSETS.md`; footprint discovery/LayoutBench schema coverage plus `multimodal-inventory-local` validates real tiny ASCII Grid/PRJ and PLY artifacts, checksums/header bounds, full CRS/epoch/provenance/lifecycle sidecars, URI policy, exact/pruned queries, and version supersession; `just multimodal-inventory-evidence-check` validates copied COG/COPC packet manifests so skipped wiring cannot be promoted | run copied COG and COPC/LAZ collections through object-store lifecycle/restore/workload/scale gates, then validate 3D/CAD/BIM families |

## Current hard blockers for production claims

- Real external PostgreSQL/S3-compatible service drills have not been run; they are
  now promotion work after local Kind+Linkerd maximum evidence is stable.
- Larger real-data matrices require copied datasets and external client systems.
- The PostgreSQL DuckLake backend is a non-spec multicatalog layout until a
  DuckDB/official-DuckLake reference-reader or tested export/migration gate proves
  a stronger claim.
- DuckDB is not yet the default pgwire query/storage path. The feature-gated local
  CLI route proves the bounded D2 workflow and policy boundary, but native
  cancellation/streaming, legacy-adapter neutrality, migration, catalog/PostGIS/
  client parity, shared concurrency, production packaging, and D1–D5 operational
  gates remain production blockers.
- Spatial family identity is durable for explicit SQL declarations, but subtype,
  SRID, dimensions, existing-blob migration, geography reference-reader behavior,
  generic PostgreSQL type fidelity, and external-profile evidence remain open.
- Native mutation crash/failure injection has local failpoint and real-process
  before/after-commit coverage for native `DELETE`/`UPDATE`/bucket compaction plus
  actual prewrite inventory and local quarantine evidence, but permanent deletion,
  generic response-loss replay, Kind, and external-service drills are not automated.
- Protected snapshots, live release switching, branch/merge, materialized views,
  and CDC row UDTFs depend on implementation work and/or upstream-stable APIs;
  managed-service rollback execution remains open beyond the local prior-head
  restore oracle.
- CDC row table functions stay disabled until pgwire/Arrow projection is safe.
- Write-side schema/table allowlists are implemented for service identities; the
  first read-side allowlist denies non-allowlisted DuckLake reads and unfiltered
  metadata surfaces, but client-compatible filtered metadata views and separate
  administrative permissions remain trace-driven future work.

## Maintenance rule

When a roadmap item gains a new contract, add it here and link the owning doc
instead of duplicating implemented details throughout `ROADMAP.md`. Update the
forward roadmap only when an exit gate, priority, or product outcome changes.
When execution evidence lands, update the relevant compatibility/operations
record and release-evidence packet for the exact source SHA.
