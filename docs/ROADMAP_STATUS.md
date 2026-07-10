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
| External PostgreSQL/S3 Alpha | `docs/ALPHA_EXTERNAL_SERVICES.md` defines credential rotation, restart, throttling, backup/restore, cleanup, and refresh drills; `just external-alpha-evidence-check` validates redacted packet manifests against collected metrics so wiring smokes cannot be mislabeled as Alpha promotion | run against real platform-managed PostgreSQL/S3 services and publish artifacts |
| Benchmark ladder | `docs/ANALYTICS_BENCHMARKS.md`; manual `Benchmark ladder` workflow; QPS/OLAP/compaction metrics; validated `layoutbench-regional-r100m-v1` defines exactly 100M rows and 202 load batches; PostgreSQL metadata-provider calls are process-metered; snapshot-fresh extended execution has a 7-call local oracle; the bounded Kind runner seeds/measures the exact profile and the profile-bound parser enforces warm/cold/direct/refresh budgets | execute the 100M, billion-row, real-data, managed-service, and release-budget ladders; physical read/write roundtrip instrumentation remains open |
| Real-data client matrix | `docs/REAL_DATA_CLIENT_MATRIX.md`; OSM Monaco opt-in baseline includes OGR copy/read, SQL sample parity, MVT SQL bytes, and QGIS open/filter/render; MVT encoder/SQL probes have key/value tags and the real Martin binary opt-in proves configured attributes on a synthetic layer | add GeoServer and real Martin binary/OSM attribute side-by-side, then wider OSM/Overture-derived layers |
| API/client expansion | `docs/API_CLIENT_PROBES.md`; `just api-client-local-smoke` is in `just ci`; `just kind-api-client-probe` is in `just kind-compatibility` and compatibility metrics for pgwire/catalog/WKB/bbox/BI/MVT profile surfaces, including MVT attribute tags | promote to named psycopg, SQLAlchemy/GeoPandas, pg_featureserv, MVT, and BI client probes over real dependencies/data |
| PostGIS conformance | `docs/POSTGIS_CONFORMANCE.md`; `just postgis-regress`; `just postgis-conformance-summary` | promote broader fixture families through pgwire/client traces |
| Native mutation safety | local native DML/compaction tests; before-commit failpoint retry oracles and abort metric; six real-process `SIGKILL` cases cover delete/update/explicit bucket compaction before and after commit, prove exact prewrite-to-inventory equality before commit, committed-path exclusion after commit, restart state, and explicit before-commit retry; offline `--orphan-inventory` remains age-gated and dry-run only; `docs/MUTATION_FAILURE_DRILLS.md` | extend the process matrix to Kind/managed-service storage, transaction batching, cleanup/quarantine/deletion, response-loss reconciliation, and reference-reader interop |
| DuckLake alignment | SQLite/local is the spec-oriented single-catalog path but is not yet drop-in DuckDB-writable; `docs/DUCKLAKE_ALIGNMENT.md` records that the current PostgreSQL multicatalog backend is library-specific/non-spec | reference-reader/export gates for both profiles plus a blocking 1.0 PostgreSQL migration decision |
| Spatial family identity | Explicit SQL geometry/geography persist as Binary WKB/EWKB plus validated Arrow metadata and snapshot-versioned DuckLake `column_type`; dynamic metadata tables, RowDescription, `pg_attribute`, and pgjdbc are metadata-first; an unconventional-name UPDATE/compaction/restart pgwire test proves OIDs and no `geom TEXT` false positive; old conventional Binary remains compatible | durable subtype/SRID/dimensions, old-blob migration, geography reference-reader interoperability, generic pg_type/typmod fidelity, and PostgreSQL/S3 external evidence |
| Spatial layout | ordinary hidden bbox/time/space/Morton columns, statistics-based pruning, exact recheck, and native bucket compaction are implemented | real-data scale, structural partition/file stats, catalog budgets, and only then possible true coarse DuckLake partitions |
| Pgwire protocol boundary | raw pre-parse rewrites, parsed hooks, type/catalog encoding, dedicated COPY, and maintained cursor paths are covered | realistic pgjdbc fetch-size/portal suspension evidence and structural replacement of remaining SQL-text classifiers |
| Snapshot/time travel | `docs/SNAPSHOT_OPERATIONS.md`; literal snapshot-id and RFC3339 timestamp `AS OF` plus named selectors, exact-id validation, deterministic timestamp resolution, count/extent parity, metadata UDTFs, no-op catalog reopen, counters, and a matched-backup local rollback oracle that validates a prior head after source advancement | run rollback against managed services; implement protected retention, live release switching, and safe CDC row UDTFs |
| Security/RBAC | SCRAM/read-only/TLS docs and tests; a structural read-only allowlist denies DDL/DML/maintenance/indeterminate statements before catalog refresh with SQLSTATE `42501`; write-capable service identities can be restricted with normalized DuckLake table allowlists and matching explicit-user privilege metadata; bounded error logs and denial metrics are covered; `docs/SECURITY_RBAC.md` defines the RBAC target | execute external secret/TLS failure drills, add read-side object filtering only from traces, and split future administrative permissions |
| Multi-modal assets | `docs/MULTIMODAL_ASSETS.md`; footprint discovery/LayoutBench schema coverage plus `multimodal-inventory-local` validates real tiny ASCII Grid/PRJ and PLY artifacts, checksums/header bounds, full CRS/epoch/provenance/lifecycle sidecars, URI policy, exact/pruned queries, and version supersession | promote copied COG and COPC/LAZ collections through object-store lifecycle/restore/scale gates, then validate 3D/CAD/BIM families |

## Current hard blockers for production claims

- Real external PostgreSQL/S3-compatible service drills have not been run; they are
  now promotion work after local Kind+Linkerd maximum evidence is stable.
- Larger real-data matrices require copied datasets and external client systems.
- The PostgreSQL DuckLake backend is a non-spec multicatalog layout until a
  reference-reader or tested export/migration gate proves a stronger claim.
- Spatial family identity is durable for explicit SQL declarations, but subtype,
  SRID, dimensions, existing-blob migration, geography reference-reader behavior,
  generic PostgreSQL type fidelity, and external-profile evidence remain open.
- Native mutation crash/failure injection has local failpoint and real-process
  before/after-commit coverage for native `DELETE`/`UPDATE`/bucket compaction plus
  actual prewrite inventory evidence, but cleanup/quarantine/deletion, generic
  response-loss replay, Kind, and external-service drills are not automated.
- Protected snapshots, live release switching, branch/merge, materialized views,
  and CDC row UDTFs depend on implementation work and/or upstream-stable APIs;
  managed-service rollback execution remains open beyond the local prior-head
  restore oracle.
- CDC row table functions stay disabled until pgwire/Arrow projection is safe.
- Write-side schema/table allowlists are implemented for service identities; read
  filtering, metadata filtering, and separate administrative permissions remain
  trace-driven future work.

## Maintenance rule

When a roadmap item gains a new contract, add it here and link the owning doc
instead of duplicating implemented details throughout `ROADMAP.md`. Update the
forward roadmap only when an exit gate, priority, or product outcome changes.
When execution evidence lands, update the relevant compatibility/operations
record and release-evidence packet for the exact source SHA.
