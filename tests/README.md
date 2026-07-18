# Test data

Current product evidence is registered under `crates/quackgis-server/tests` and
run through `just ci` or the pinned native recipes.

`duckdb_wire_read::current_duckdb_transport_profile` owns the required 100k-row
direct DuckDB/ADBC/pgwire benchmark smoke and writes its manifest under `.tmp`.

`roadmap_profiles::offline_recovery_profile` owns the actual-process exact-path
checkpoint backup/restore gate, including format-v2 native-runtime identity
binding. `roadmap_profiles::mixed_release_workload_profile` owns the
duration-controlled M5 read/COPY/mutation/cancel/compaction/restart oracle;
smoke/local use the same implementation, while reference mode requires exactly
24 hours.

`scripts/postgis_migration_smoke.py` owns the first G0 actual-process source-to-
target gate and runs through `just postgis-migration-smoke`. It uses a digest-
pinned PostgreSQL 18/PostGIS 3.6 source plus a fresh actual QuackGIS process,
checks repeatable-read exclusion under a concurrent source write, exact
scalar/Point/NULL checksums after target reconnect, all-table rollback on invalid
input, pre-target key/report-digest rejection, report-bound staging cleanup,
identical retry checksums, and atomic promotion. The
separate `just kind-postgis-migration-gate` executes the immutable migrator through
a dedicated migration certificate, credential-bound `migration_operator` lease,
mutual-TLS tiny client, and iroh worker; it verifies 10,004 rows and denies the
ordinary K0 certificate. The gate now uses fresh staging, exact report-bound
verification and promotion, clean runtime manifest/source/image identity, a full
K0 restart, and psql/psycopg/OGR/QGIS promoted-data reads. The same gates now
assert exact source role/grant mappings, bounded progress terminal/rollback/
rejection state, and Point family/SRID/dimensions, structural WKB validity,
empty/invalid counts, and finite extents. Broader semantic validity and
interruption/resource scale remain open.

`iroh_direct::duckdb_pgwire_oracles_pass_through_local_iroh` owns the first native
I0 direct-path smoke. It uses real bootstrap, worker, and tiny-client endpoints
and proves typed/spatial query, COPY, rollback, cancellation/quarantine, and fresh
reconnect behavior against a fresh local DuckLake worker. Run it with
`just iroh-duckdb-smoke`; it is functional evidence, not relay or resource proof.

- `duckdb_spatial_compat.json` is the maintained 57-case disposition ledger; 43
  cases currently execute through pgwire.
- `fixtures/duckdb_catalog_contract.json` is the client-neutral executable
  catalog/type fixture. It covers DuckDB-derived table/column metadata, bounded
  relational spatial type catalogs, PostgreSQL 18 lookup result types, geometry
  RowDescription, binary/text WKB, NULL, unknown OIDs, and ordinary equivalent
  catalog rows/types through pgwire. It also proves namespace owner resolution and
  that ordinary `oid`/`typtype` aliases retain their native values and types.
  Schema version 4 adds implicit `pg_catalog` lookup, profile/QGIS-required
  built-ins, PostGIS-shaped spatial scalar/arrays, full-row oracle equality,
  reciprocal namespace/owner/array/collation references, explicit OID parameters,
  and fail-closed private/unsupported routing and provenance shapes.
  Schema version 5 adds stable logical-database/owner identity and structural
  `current_database`/`current_schema`/`current_schemas` behavior with exact
  PostgreSQL `name` and `name[]` wire types.
  Schema version 6 adds the bounded relational `pg_proc` identity for the four
  maintained PostGIS version routines and executes OGR's captured namespace
  lookup with reciprocal namespace validation.
- `fixtures/postgresql18_compatibility_profile.json` freezes the first target
  catalog/query/wire contract and names every still-pending client trace.
  `fixtures/postgresql18_column_core_reference.json` records normalized
  RowDescription/type evidence from the digest-pinned PostgreSQL 18.4 oracle.
  `just project-contract-check` validates both files together; they are target
  contracts, not QuackGIS implementation claims.
  The profile now also records source-executable captured query families: psql
  `resolve_relation`, QGIS `attribute_structure`, OGR `column_structure`, and
  OGR's truthfully empty `primary_key_columns`. The pinned identity test reads
  their SQL from the trace fixtures and proves exact wire types and rows; all
  other trace statements remain targets unless separately named as evidence.
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
