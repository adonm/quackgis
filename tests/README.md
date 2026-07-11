# Test data

Current product evidence is registered under `crates/quackgis-server/tests` and
run through `just ci` or the pinned native recipes.

`duckdb_wire_read::current_duckdb_transport_profile` owns the required 100k-row
direct DuckDB/ADBC/pgwire benchmark smoke and writes its manifest under `.tmp`.

- `duckdb_spatial_compat.json` is the maintained 57-case disposition ledger; 42
  cases currently execute through pgwire.
- `fixtures/postgis_curated_cases.rs` is the deliberately bounded expected-value
  source parsed by the native CLI and pgwire gates, not a Cargo integration test.

Historical and broad upstream corpora live in Git history, not the active test
tree. Add a case only when a maintained client/workload requires it, route it
through the current DuckDB-only server, and assign an implementation disposition.
