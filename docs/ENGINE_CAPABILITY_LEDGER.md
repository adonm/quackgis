# Engine capability ledger

This ledger is the D0 migration inventory between the current
DataFusion/SedonaDB/`datafusion-ducklake` backend and the DuckDB/official-DuckLake
target. It records ownership and parity gates; it is not evidence that the DuckDB
route is complete. Exact promotion evidence remains in the linked documents and
must identify a source SHA and promotion ring.

Status meanings:

- **preserve** — same externally visible contract;
- **replace** — engine/storage implementation changes behind a stable edge;
- **redesign** — behavior remains, but the internal contract must change;
- **defer** — explicitly outside the first DuckDB cutover;
- **blocked** — no DuckDB server route or required external evidence yet.

## Boundary state

- `--engine-backend=legacy-datafusion|duckdb` makes selection explicit. The
  default remains `legacy-datafusion`; a `duckdb-adbc` build enables the bounded
  local DuckDB CLI route, while binaries without that feature fail before roots.
- `engine::ServerEngine` owns backend-specific startup. Only the legacy branch
  returns a DataFusion `SessionContext`, so D0 remains open outside the adapter.
- `engine_api` now defines DataFusion-free Arrow query/result, prepared-statement
  description, table, ingest, snapshot, maintenance, and classified-error types.
  The DuckDB ADBC kernel implements that contract; the legacy pgwire path has not
  yet migrated to it. ADBC query batches remain materialized, although pgwire row
  encoding no longer creates a second fully collected row buffer.
- every write-capable server start atomically creates or validates
  `_quackgis/storage-authority-v1` under the data prefix. The marker contains only
  a version and authority id. A different authority fails before catalog
  initialization or object writes.
- direct ADBC evaluation claims only local roots for
  `duckdb-official-ducklake`. Remote ADBC roots fail closed until shared credentials
  and marker handling are wired through the production adapter.
- the ADBC native boundary verifies the exact supported shared-library SHA-256 and
  SQL runtime version before claiming a root; modified/mixed native artifacts fail
  before storage initialization.
- migration/export must use a distinct destination root. Never copy the authority
  marker from a legacy root into a DuckDB destination. Backup/restore of the same
  authority must preserve it.

## Capability inventory

| Capability | Owner after cutover | Disposition | Legacy oracle | DuckDB parity gate / current state |
|---|---|---|---|---|
| pgwire simple query | Rust pgwire edge | preserve | `wire_spatial`, preview smoke | feature-gated CLI SELECT/CREATE/INSERT/UPDATE/DELETE/transactions traverse pgwire→ADBC→DuckDB→official DuckLake→Arrow→pgwire with structural single-statement admission; catalog/PostGIS/client breadth remains blocked |
| extended parse/bind/describe/execute | Rust pgwire edge | preserve | `wire_extended_protocol_describe`, API client probe | Parse/Describe/bound SELECT plus parameterized INSERT/UPDATE/DELETE, empty RowDescription, typed errors, and independent sessions pass through ADBC; cancellation and broader parameter types remain blocked |
| cursors and portal/fetch behavior | Rust pgwire edge | preserve/redesign | cursor tests plus raw `Execute.max_rows` suspend/resume/final-page oracle | DuckDB named portal now returns three ordered `max_rows=1` pages through suspend/resume/final completion; realistic pgjdbc sizes, query streaming and SQL cursor statements remain |
| PostgreSQL text/binary result encoding | Rust pgwire edge | preserve | `wire_spatial`, OGR/QGIS traces | DuckDB reuses `arrow-pg`, proves maintained scalar RowDescription/rows, and lazily encodes materialized batches without a second row collection; full spatial/client matrix remains blocked |
| COPY FROM STDIN | Rust edge + engine Arrow ingest | redesign | persistence and OGR COPY tests | bounded text pgwire COPY converts BOOL/SMALLINT/INT/BIGINT/REAL/DOUBLE/DECIMAL/DATE/TIMESTAMP/VARCHAR/`\\x` WKB to Arrow, ingests through ADBC, preserves exact spatial/reopen bytes, rolls back transactionally, and applies write policy before schema lookup; options/escaping, more types, batch backpressure, and OGR remain |
| TLS and SCRAM | Rust edge | preserve | malformed configured TLS now fails without plaintext fallback; auth/TLS lifecycle and client probes | real DuckDB CLI process proves SCRAM writer/reader startup; DuckDB TLS plus external plaintext-denial/rotation evidence remain |
| read/write allowlists | Rust policy edge | preserve | auth and wire tests | real DuckDB CLI process applies normalized write/read policy before ADBC prepare and denies reader INSERT/COPY, non-allowlisted tables, and unfiltered DuckLake metadata with `42501`, metrics, and redacted audit events |
| audit and metrics | Rust control plane | redesign | audit/metrics unit and probe gates | add DuckDB IO, cancellation, retry, spill, and maintenance outcomes; blocked |
| `pg_catalog`/`information_schema` shims | Rust compatibility edge | preserve/redesign | pgjdbc/OGR/QGIS/GeoServer traces | derive real metadata from DuckDB and retain trace-required synthetic rows; blocked |
| geometry/geography OIDs and RowDescription | Rust compatibility edge | preserve | explicit-family persistence/wire tests | exact OID and binary-format parity after DuckDB reopen; blocked |
| WKB/EWKB durable bytes | Rust contract + DuckDB storage | preserve | persistence, OGR/QGIS, regress tests | valid WKB ADBC ingest + exact DuckDB spatial/reopen passes in required CI, followed by an independent CLI reopen of the same ADBC-authored catalog; EWKB/client parity remains |
| maintained `ST_*` scalar predicates | DuckDB spatial first; Rust aliases/gaps | replace | curated PostGIS regress and SedonaDB oracle | all 57 cases are classified: 31 native, 5 rewrite, 4 macro, 12 Rust edge, 5 extension candidates; all 40 executable cases match through the real DuckDB pgwire server, while edge/extension cases and broader fixtures remain blocked |
| CRS/projection and geography semantics | DuckDB spatial + small bounded gaps | redesign | regress/client suites | versioned semantic matrix; blocked |
| MVT and aggregate compatibility | Rust/DuckDB extension boundary | redesign | Martin and OSM attribute probes | real Martin/client parity; blocked |
| DDL and INSERT | DuckDB official DuckLake | replace | persistence/wire tests | bounded pgwire CREATE and literal/parameterized INSERT author official DuckLake under table policy and survive reopen; broad DDL/client fidelity remains blocked |
| UPDATE/DELETE | DuckDB official DuckLake | replace | native mutation + six process-kill cases | literal and parameterized pgwire UPDATE/DELETE plus ADBC transactions pass locally under policy; crash/conflict/response-loss remain blocked |
| explicit transactions | DuckDB connection/session adapter | redesign | staged single-table wire tests | bounded pgwire clients now own independent ADBC sessions; BEGIN/ROLLBACK/COMMIT proves cross-client isolation and post-commit visibility, while disconnect rolls back active work. Multi-table limits, conflicts, and indeterminate outcomes remain |
| snapshots and `AS OF` wrappers | Rust control plane over DuckLake metadata | redesign | snapshot operations and rollback oracle | typed official snapshot id/timestamp discovery passes locally; `AS OF`, retention, and restart wrapper parity remain blocked |
| compaction/retention/cleanup | DuckDB DuckLake + Rust policy | replace/redesign | native compaction/orphan tests | typed official merge/rewrite requests exist and adjacent-file merge preserves rows locally; crash safety, protected retention, and cleanup remain blocked |
| local catalog/files profile | DuckDB official DuckLake | replace | SQLite persistence suite | feature-gated real CLI route and independent authority/reopen slices pass; legacy migration remains blocked |
| PostgreSQL/object-storage profile | DuckDB official DuckLake | replace | Kind/external multicatalog probes | shared official profile, authority marker, conflict/recovery and managed evidence; blocked |
| hidden bbox/time/Morton layout | DuckDB columns/stats/macros | redesign | LayoutBench exact-vs-pruned tests | official DuckLake WKB+bbox oracle proves 3 conservative candidates, 2 exact polygon-hole hits, both predicates in DuckDB plan, and reopen equality; time/Morton maintenance and scale budgets remain blocked |
| cancellation and resource limits | Rust adapter + DuckDB | redesign | process shutdown only | query cancellation, memory/spill/admission tests; blocked |
| legacy catalog export/import | migration tool | replace | current SQLite/PostgreSQL fixtures | schema/count/WKB/layout/checksum parity, declared snapshot reset, rollback; blocked |
| QGIS, OGR, GeoServer, Martin | Rust compatibility edge | preserve | maintained local/Kind traces | repeat all traces against DuckDB backend; blocked |
| Python/API/BI clients | Rust compatibility edge | preserve | API client profile probe | named real clients and copied data; blocked |
| multi-modal sidecar inventory | Rust SQL contract | preserve | tiny raster/PLY inventory oracle | rerun unchanged after cutover; real COG/COPC evidence remains external |
| PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication | none | defer | explicitly unsupported | remains out of scope unless product direction changes |

## D0 closure checklist

- [x] Target direction and preserve/replace/redesign/defer inventory exist.
- [x] Backend selection is typed, explicit, and fail-closed.
- [x] Local and configured object-store data roots have an atomic writer-authority
  marker for write-capable server paths.
- [x] Current server startup runs through `ServerEngine`.
- [ ] Query/result streaming, parameter, transaction, schema, snapshot,
  maintenance, cancellation, and error types are engine-neutral.
- [ ] Pgwire handlers no longer own DataFusion planning/execution directly.
- [ ] Every blocked row above has an implemented DuckDB parity test rather than
  only a named gate.

D0 remains open until the unchecked items close. Update this ledger in the same
commit as each new adapter contract or parity test.
