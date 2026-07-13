# Test data

Current product evidence is registered under `crates/quackgis-server/tests` and
run through `just ci` or the pinned native recipes.

`duckdb_wire_read::current_duckdb_transport_profile` owns the required 100k-row
direct DuckDB/ADBC/pgwire benchmark smoke and writes its manifest under `.tmp`.

- `duckdb_spatial_compat.json` is the maintained 57-case disposition ledger; 42
  cases currently execute through pgwire.
- `fixtures/duckdb_catalog_contract.json` is the client-neutral executable
  catalog/type fixture. It covers DuckDB-derived table/column metadata, bounded
  relational spatial type catalogs, PostgreSQL 18 lookup result types, geometry
  RowDescription, binary/text WKB, NULL, unknown OIDs, and ordinary equivalent
  catalog rows/types through pgwire. It also proves namespace owner resolution and
  that ordinary `oid`/`typtype` aliases retain their native values and types.
- `fixtures/postgresql18_compatibility_profile.json` freezes the first target
  catalog/query/wire contract and names every still-pending client trace.
  `fixtures/postgresql18_column_core_reference.json` records normalized
  RowDescription/type evidence from the digest-pinned PostgreSQL 18.4 oracle.
  `just project-contract-check` validates both files together; they are target
  contracts, not QuackGIS implementation claims.
- `fixtures/ogr_3_11_5_postgresql18_trace.json` is a credential-free normalized
  trace from the exact pinned OGR image against digest-pinned PostgreSQL
  18.4/PostGIS. It freezes copied-point discovery SQL and observed results; it does
  not claim those queries execute through QuackGIS yet.
- `fixtures/psql_18_3_postgresql18_describe_trace.json` freezes the 12 normalized
  query families and rendered structure from exact psql 18.3 `\d+` against the
  same PostgreSQL 18.4/PostGIS oracle. It is a target corpus, not current QuackGIS
  describe support.
- `fixtures/qgis_3_44_postgresql18_trace.json` freezes 32 statements (26 unique
  families) from an offscreen QGIS 3.44.11 PostgreSQL provider run that opens,
  inspects, counts, measures, and reads a spatial layer. The digest-pinned oracle
  succeeds; QuackGIS support for the traced surface remains staged work.
- `fixtures/postgis_curated_cases.rs` is the deliberately bounded expected-value
  source parsed by the native CLI and pgwire gates, not a Cargo integration test.

Historical and broad upstream corpora live in Git history, not the active test
tree. Add a case only when a maintained client/workload requires it, route it
through the current DuckDB-only server, and assign an implementation disposition.
