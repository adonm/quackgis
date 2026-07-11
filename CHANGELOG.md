# Changelog

QuackGIS has not published a stable release. Prototype-era details and commit
anchors live in [docs/HISTORY.md](./docs/HISTORY.md) and Git history.

## Unreleased — DuckDB-only local preview

### Added

- Owned Rust pgwire/TLS/SCRAM edge over DuckDB ADBC.
- Official local DuckLake create, Arrow ingest/query, transaction, snapshot
  inspection, adjacent-file merge, and reopen workflows.
- Structural single-statement admission and parsed read/write table policy.
- Parameterized reads/mutations, bounded PostgreSQL text COPY, independent client
  sessions, transaction cleanup, and portal paging.
- DuckDB Spatial execution with 42 curated original-PostGIS expressions routed
  through native functions or bounded server-owned rewrites/macros.
- Checksum/version validation for `libduckdb` and signed `spatial`/`ducklake`
  extensions, plus a load-only runtime image contract.
- DataFusion-free Arrow-to-pgwire encoder with maintained WKB sentinel identity.

### Changed

- DuckDB and official DuckLake are unconditional and are the sole engine/storage
  authority for new roots.
- Project direction now prioritizes bounded streaming, cancellation, admission,
  bulk ingest, focused named clients, and measured spatial performance before
  shared or broad compatibility claims.
- Missing capabilities follow the native DuckDB → SQL macro/rewrite → Rust edge →
  vectorized DuckDB extension decision ladder.
- CI bootstraps and verifies the pinned native runtime before native gates.
- Runtime artifacts package the complete DuckDB bundle rather than a bare server
  binary.

### Removed

- DataFusion, Sedona SQL execution, `datafusion-postgres`,
  `datafusion-pg-catalog`, and forked `datafusion-ducklake` runtime/vendor trees.
- Legacy engine modules, catalog hooks, native writer/mutation code, and associated
  tests.
- Optional backend selection and the `duckdb-adbc` feature gate.
- Unsupported shared PostgreSQL/S3 deployment automation and stale Kind/client
  probes.
- Nonfunctional examples, benchmark runners, migration plans, and scheduled jobs
  that depended on retired code or absent tests.

### Known limits

- Materialized query results and 16 MiB buffered COPY.
- No native cancellation, resource admission, or productized memory/spill policy.
- Local official DuckLake only.
- Incomplete catalogs, geometry identity, spatial gaps, and named GIS clients.
- No production backup/restore, upgrade, soak, or shared-profile evidence.
