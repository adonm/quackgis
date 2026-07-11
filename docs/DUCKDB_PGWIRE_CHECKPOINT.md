# DuckDB pgwire local backend

The feature-gated local backend proves the intended D2 center seam through the
real server CLI without making DuckDB the default:

```text
pgwire TLS/SCRAM/startup/error/cancel framing
→ structural single-statement admission + normalized table policy
→ bounded simple/extended query, DDL/DML, COPY, or transaction
→ ADBC prepare/describe/Arrow bind
→ DuckDB + official DuckLake
→ Arrow schema/materialized batches
→ existing PostgreSQL OID text/binary encoder
→ lazy pgwire row stream
```

Run the complete real-driver gate after the pinned native bootstrap:

```sh
mise run duckdb-bootstrap
mise exec -- just duckdb-pgwire-workflow-test
```

Run the bounded backend manually on separate roots:

```sh
mkdir -p .tmp/duckdb-server
mise exec -- cargo run -p quackgis-server --features duckdb-adbc -- \
  --engine-backend=duckdb \
  --catalog-path=.tmp/duckdb-server/catalog.ducklake \
  --data-path=.tmp/duckdb-server/data
```

`QUACKGIS_DUCKDB_ADBC_DRIVER` is provided by `mise.toml`. Fail-closed startup
uses `LOAD` only, verifies the exact `libduckdb` digest and SQL runtime version,
requires local catalog/data paths, and refuses a binary built without the feature.
Legacy and DuckDB roots must never be shared.

## Implemented local contract

The real-driver workflow proves:

- the actual `quackgis-server --engine-backend=duckdb` process opens and serves an
  official local DuckLake;
- PostgreSQL AST classification admits one bounded SELECT, CREATE TABLE, INSERT,
  UPDATE, DELETE, BEGIN, COMMIT, or ROLLBACK and rejects unapproved/multiple
  statements; comments, CTEs, and literal semicolons do not affect routing;
- extended Parse/Describe/Bind/Execute supports typed parameterized SELECT and
  INSERT/UPDATE/DELETE without SQL interpolation;
- empty results retain RowDescription; Boolean/integer/float/decimal/date/
  timestamp/null schemas map through the maintained PostgreSQL encoder;
- encoded rows are yielded directly from Arrow batches instead of collecting a
  second complete row vector (ADBC results themselves remain materialized);
- a binary WKB parameter drives exact DuckDB `ST_Intersects`;
- all 40 native/rewrite/macro entries in the maintained 57-case spatial ledger
  execute through pgwire and match the curated scalar oracle;
- named portals return three ordered `max_rows=1` pages with suspension/resume and
  final completion;
- bounded text COPY parses Boolean, SMALLINT/INTEGER/BIGINT, REAL/DOUBLE, DECIMAL,
  DATE, TIMESTAMP, VARCHAR, and PostgreSQL `\\x` Binary/WKB into Arrow, preserves
  exact spatial/reopen bytes, and rolls back inside an explicit transaction;
- each client owns an independent ADBC session: uncommitted changes stay isolated,
  rollback restores prior state, commit becomes visible, and disconnect rolls back;
- SCRAM writer/reader startup works on the real CLI process; normalized read/write
  allowlists run before ADBC prepare/schema access and deny reader INSERT, COPY,
  non-allowlisted tables, and unfiltered DuckLake metadata with SQLSTATE `42501`,
  counters, and redacted audit events; and
- official snapshot inspection works through pgwire, and CLI-authored state
  survives shutdown/reopen.

## Deliberate limits

The backend remains a bounded local migration profile. It materializes ADBC Arrow
batches, buffers each COPY request up to 16 MiB, and does not yet propagate pgwire
cancellation into a native ADBC statement. COPY options, PostgreSQL text escaping,
arrays/JSON/time zones and remaining types are unsupported. Catalog/PostGIS shims,
geometry/geography OIDs, Rust-edge/extension spatial cases, shared PostgreSQL/object
storage, orphan operations, named maintained clients, and resource/soak evidence
remain release blockers. The default backend therefore remains
`legacy-datafusion`.
