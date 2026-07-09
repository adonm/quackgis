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
- Snapshot metadata inspection and narrow single-table time travel through named
  selectors and `AS OF SNAPSHOT <id>` preprocessing.

### Changed

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
- Geometry discovery still relies partly on conventional column names and
  sentinel OIDs; durable DuckLake geometry identity remains forward work.
- General extended-protocol portal/fetch-size suspension is not implemented.
- Explicit transactions are intentionally single-table and do not provide general
  read-your-writes SELECT or multi-table atomicity.
- Timestamp time travel, protected releases, CDC rows, object-level RBAC, managed-
  service failure drills, and real multi-modal inventories remain roadmap work.

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
