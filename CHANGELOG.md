# Changelog

This changelog tracks the current Rust pgwire architecture. QuackGIS has not yet
published a Git release tag; old `v0.1`/`v0.2` names were internal prototype eras,
not stable releases. See [docs/HISTORY.md](./docs/HISTORY.md) for the architectural
story, useful commit anchors, and lessons for contributors.

## Unreleased — Rust pgwire spatial lakehouse preview

### Added

- A single Rust `quackgis-server` that exposes PostGIS-compatible SQL over pgwire
  without running PostgreSQL or DuckDB as the query engine.
- DataFusion + SedonaDB query/spatial execution and DuckLake/Parquet persistence
  for SQLite/local and PostgreSQL/object-storage profiles.
- CTAS, CREATE/ALTER-add-column, INSERT, PostgreSQL text `COPY FROM STDIN`,
  UPDATE/DELETE, `RETURNING`, single-table staged transactions, and explicit
  compaction.
- Fork-backed atomic positional DELETE, UPDATE replacement rows, and bucket
  compaction under one visible DuckLake snapshot.
- WKB/EWKB wire handling, sentinel geometry/geography OIDs, PostGIS metadata,
  spatial functions/adapters, MVT output, and trace-driven catalog compatibility.
- QGIS read/edit, OGR load/read, GeoServer WFS/WMS/WFS-T, Martin, PostGIS regress,
  API/client-profile, and real OSM comparison gates.
- Hidden bbox/time/space/Morton layout columns, safe statistics-based candidate
  pruning, exact SedonaDB recheck, LayoutBench, and scan/latency budgets.
- Shared-catalog Kind Alpha gates for multi-pod reads/writes, QPS, OLAP, Linkerd
  mTLS visibility, conflict/retry, native mutation metadata, and metrics reports.
- SCRAM password mode, optional TLS, coarse read-only/read-write authorization,
  safe metrics, backup/restore oracles, and operations/security runbooks.
- Fail-closed structural read-only statement authorization with PostgreSQL
  insufficient-privilege errors, bounded pgwire error logging, and Ctrl-C/SIGTERM
  process lifecycle tests.
- A machine-validated `layoutbench-regional-r100m-v1` contract with exact table/
  batch arithmetic, bounded generation, catalog provider-call/refresh budgets, and
  required evidence metadata; the local runner rejects ambiguous `sf1` naming.
- Valid tiny raster (ASCII Grid/PRJ) and point-cloud (PLY) fixtures with a pinned
  sidecar manifest and pgwire tests for checksum/header bounds, CRS/epoch/
  provenance, URI policy, exact pruning, and asset-version lifecycle.
- Offline, dry-run-only orphan inventory with a mandatory age cutoff, redacted
  count-only default, explicit path opt-in, missing-catalog refusal, and
  cross-catalog PostgreSQL reference semantics.
- Private filesystem barriers and six Unix subprocess `SIGKILL` tests around native
  delete/update/bucket-compaction commit: before-commit prewrites exactly match the
  real offline orphan inventory, while after-commit paths remain referenced and
  restart exposes the committed state without blind replay.
- Snapshot metadata inspection and narrow single-table time travel through named
  selectors, exact snapshot-id validation, and snapshot-id/RFC3339 timestamp
  `AS OF` preprocessing and resolution.
- Matched-backup rollback validation that records a release snapshot, advances the
  source, and verifies the isolated prior head, current rows, simple-protocol
  `AS OF`, and referenced files.
- Real Martin binary opt-in coverage for configured MVT attribute propagation on
  the deterministic synthetic layer, including catalog property discovery and
  record-form `ST_AsMVT` attribute expansion.
- Durable explicit geometry/geography family identity: SQL declarations remain
  Binary WKB/EWKB, carry validated Arrow field metadata, persist as snapshot-
  versioned DuckLake `column_type`, and drive live metadata/OID surfaces across
  rewrites and restart without classifying `geom TEXT` as spatial.
- Process-local PostgreSQL catalog read-provider-call instrumentation with safe
  Prometheus export, success/error fake-provider tests, and schema-only pruning
  preflight that keeps snapshot-fresh extended execution at 7 provider calls.
  Extended prepared statements now replan before parameter binding, including
  selective reads with unrelated bind parameters, so appends after prepare are
  visible instead of retaining the parse-time file set.
- A profile-bound `layoutbench_catalog` report generator that rejects missing,
  duplicate, malformed, incorrect, inconsistent, or over-budget cold/direct/warm
  phases and emits trend/budget-compatible provider-call metrics without claiming
  wire-level roundtrips. Actual 100M and managed-service execution remain open.
- Explicit offline orphan quarantine planning/apply for old unreferenced Parquet
  candidates, with copy-before-remove, live-prefix rejection, destination overwrite
  refusal, and a local test proving referenced DuckLake files stay in place.
- Real OSM parity now checks SQL-MVT `name` attribute tokens and exports OSM row,
  GeoJSON, QGIS, MVT byte, and MVT attribute fields into compatibility metrics.
- External Alpha evidence manifests now require restore RPO/RTO fields,
  failed-writer quarantine fields, and a migration implication for the current
  non-standard PostgreSQL multicatalog interoperability result.
- A copied multi-modal inventory evidence checker validates COG/raster plus
  COPC/LAZ point-cloud promotion packets, non-secret URI policy, lifecycle,
  restore, workload, and maintained vector-gate evidence before roadmap claims.
- Documented a DuckDB/ADBC evaluation ladder and manifest checker that make
  DuckDB's official DuckLake extension the preferred reference-reader gate before
  any storage-writer or engine pivot.
- Added `just duckdb-engine-probe`, an optional out-of-process DuckDB CLI smoke for
  the DuckDB `spatial` and `ducklake` extensions before deeper engine-pivot work.
- Added a feature-gated DuckDB ADBC storage-kernel slice with official DuckLake
  attach, valid-WKB Arrow ingestion, exact spatial query/reopen, one-snapshot
  transactions, rollback, fail-fast reentrant access, unsafe-connection quarantine,
  and a real libduckdb integration test; ordinary CI compiles the optional path and
  the default server storage path is unchanged.
- Pinned DuckDB CLI 1.5.4 through mise/Aqua and added a Linux x86_64 bootstrap that
  verifies official libduckdb release/library checksums, preinstalls signed
  `ducklake` and `spatial` extensions in an isolated local home, records artifact
  digests, and runs native ADBC/CLI probes with network-free `LOAD` only locally
  and in required pull-request CI.
- Added a DataFusion-free Arrow engine contract and implemented it for the DuckDB
  ADBC kernel, including prepared describe/bind, empty-result schemas, table
  discovery, typed snapshots, official adjacent-file maintenance, classified
  errors, and independent CLI reopen of the same ADBC-authored catalog.
- Promoted the ADBC kernel to open independent sessions from one process-owned
  database and added a real-driver isolation oracle proving uncommitted changes
  stay connection-local and become visible to another session only after commit.
- Added a required-CI DuckDB spatial classifier covering all 57 maintained pgwire
  PostGIS cases: 31 execute natively, 5 via mechanical rewrites, 4 via explicit
  compatibility macros, 12 remain Rust-edge behavior, and 5 are extension
  candidates; all 40 executable cases match the pinned engine.
- Added a separate digest-pinned Linux x86_64 DuckDB evaluation image: context
  preparation verifies committed native artifact checksums and records provenance,
  the static gate rejects online installs, and CI builds the image then loads both
  extensions with container networking disabled.
- Hardened the in-process native trust boundary to hash `libduckdb.so` and require
  exact DuckDB SQL runtime version `v1.5.4` before claiming or creating a storage
  root; modified and mixed-version artifacts fail closed.
- Added a DuckDB/official-DuckLake hidden-layout oracle proving bbox candidates
  conservatively over-select a polygon-hole result, the DuckDB plan retains the
  exact `ST_Intersects` recheck, and reopen preserves equality.
- Added a bounded DuckDB pgwire handler and real-driver gate covering simple and
  extended routing, empty-result metadata, existing Arrow-to-PostgreSQL encoding,
  and explicit unsupported-shape refusal.
- Extended that checkpoint with fully qualified autocommit `CREATE TABLE`,
  `INSERT`, `UPDATE`, and `DELETE`, plus shutdown/reopen persistence; explicit
  transactions now use per-client ADBC sessions and prove cross-client isolation,
  rollback, commit visibility, and disconnect rollback.
- Completed the bounded local D2 workflow with text `COPY FROM STDIN` converted to
  Arrow (including PostgreSQL hex WKB), transactional COPY rollback, binary-WKB
  parameterized exact spatial SELECT, official snapshot inspection, and exact
  WKB/count persistence after restart.
- Replayed extended portal paging on DuckDB with three ordered `max_rows=1` pages,
  preserving pgwire suspend/resume and final completion behavior.
- Proved DuckDB Arrow boolean, integer, float, decimal, date, timestamp, and nullable
  schemas map to maintained PostgreSQL RowDescription types and encode a real row.
- Added explicit `legacy-datafusion|duckdb` backend selection, a startup engine
  adapter, an initial capability/parity ledger, and atomic per-data-root writer-
  authority markers. Feature-gated builds now expose the bounded local DuckDB CLI
  route; the default remains legacy.
- Promoted that route through the actual server process with structural SQL
  admission, parameterized INSERT/UPDATE/DELETE, SCRAM writer/reader startup,
  normalized read/write allowlists before ADBC prepare, denial audit/metrics, and
  local official-DuckLake restart evidence.
- Expanded DuckDB text COPY to Boolean, SMALLINT/INTEGER/BIGINT, REAL/DOUBLE,
  DECIMAL, DATE, TIMESTAMP, text, and Binary/WKB Arrow columns, and removed the
  second fully collected pgwire row buffer while retaining bounded ADBC batches.
- Routed all 40 native/rewrite/macro entries in the maintained 57-case spatial
  ledger through the real DuckDB pgwire server and compared their scalar results
  with the curated PostGIS oracle.
- Configured pgwire TLS now fails startup on unreadable, malformed, or mismatched
  certificate/key material instead of logging a warning and serving plaintext.
- Initial DuckDB evaluation evidence: DuckDB v1.5.2 passed the out-of-process
  spatial/WKB + DuckLake extension smoke, but failed to attach the QuackGIS preview
  SQLite DuckLake catalog through the documented `ducklake:sqlite:` URI because
  DuckDB expects allocator metadata columns absent from the current
  `datafusion-ducklake` writer path.

### Changed

- Made DuckDB + official DuckLake the explicit roadmap target for the query/storage
  center, with a staged D0–D5 redesign that preserves the Rust pgwire/PostGIS,
  client, spatial, security, operations, and migration contracts before retiring
  the current DataFusion/SedonaDB/`datafusion-ducklake` path.
- Retired the original DuckDB extension and the PostgreSQL + pg_ducklake + C
  geometry-extension facade. Their useful compatibility ideas were reimplemented
  at explicit Rust pgwire/catalog boundaries.
- Vendored `datafusion-postgres` and `datafusion-ducklake` where QuackGIS needs
  parser-boundary and atomic-mutation behavior. Divergence is recorded in
  `DIVERGENCE.md` and `docs/DUCKLAKE_ALIGNMENT.md`.
- Reframed the roadmap around managed-service, city, regional, dataset-release,
  multi-modal, and national-scale product outcomes with measurable exit gates.
- Split documentation authority: roadmap for future outcomes, architecture for
  invariants, status for implemented evidence, and this changelog for release
  deltas.

### Current pre-release boundaries

- The PostgreSQL DuckLake multicatalog backend is library-specific/non-spec until
  reference-reader or tested export/migration evidence exists.
- SQLite is deterministic and spec-oriented but not yet a drop-in DuckDB-writable
  catalog.
- Explicit geometry/geography family identity is durable, while conventional
  binary names remain fallback for old `blob` catalogs. Subtype/SRID/dimensions,
  old-blob migration, geography reference-reader interop, generic pg_type/typmod,
  and external PostgreSQL/S3 evidence remain forward work.
- Raw extended-protocol `Execute.max_rows` now proves bounded portal pages,
  repeated suspension/resume, and final completion; realistic pgjdbc fetch-size
  evidence remains open.
- Explicit transactions are intentionally single-table and do not provide general
  read-your-writes SELECT or multi-table atomicity.
- Protected releases, CDC rows, object-level RBAC, managed-service failure drills,
  real multi-modal inventory execution, and permanent deletion after quarantine
  remain roadmap work.

## Historical prototype eras

These eras were important experiments but are not supported release lines:

| Date | Era | What it taught us |
|---|---|---|
| 2026-06-28 | DuckDB WKB spatial extension prototype | Vectorized WKB/GeoRust execution and a PostGIS-like SQL surface were viable; the real SedonaDB bridge remained future work. |
| 2026-06-29 | DuckLake + Sedona bridge exploration | Literal SedonaDB bridging, snapshot storage, and spatial layout belonged in the design, but the first plan was too broad and speculative. |
| 2026-07-04 | PostgreSQL facade prototype | The facade explored client compatibility but was not validated end to end; PostgreSQL, pg_ducklake, DuckDB, GDAL, and a C geometry type exposed the wrong operational/ABI stack. |
| 2026-07-05 onward | Rust pgwire architecture | Keep PostgreSQL/PostGIS at the compatibility edge; own explicit protocol/catalog boundaries and use DataFusion + SedonaDB + DuckLake directly. |

Do not copy commands, deployment manifests, or support claims from a historical
era into current work. Use them only to understand a decision or recover a useful
test idea.
