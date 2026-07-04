# QuackGIS facade tests

Black-box tests that exercise the QuackGIS container through real PostgreSQL
clients (`psql`, `psycopg`), exactly the way users access it.

## Structure

```
container/
  run-all-tests.sh           — unified runner: builds, starts, tests, reports
  smoke-test.sh              — basic spatial function check (6 checks)
  test-compat.sh             — PostGIS compatibility surface (20 checks)
  test-ducklake.sh           — DuckLake storage + persistence + pruning (5 phases)
  run-postgis-fixtures.sh    — PostGIS SQL fixture runner
  tests/
    test_psycopg.py          — Python client: prepared statements, params, BI metadata
    postgis-fixtures/        — PG-syntax spatial SQL fixtures
      01_constructors.sql
      02_predicates.sql
      03_operators.sql
      04_measurements.sql
      05_overlay.sql
      06_srid.sql
```

## Running

```sh
# Build image + run all facade tests
./container/run-all-tests.sh

# Skip the image build (use existing image)
./container/run-all-tests.sh --no-build

# Skip Python tests (if psycopg not installed)
./container/run-all-tests.sh --skip-psycopg
```

Individual suites can also be run directly if a container is already running:

```sh
PG_PORT=55432 PG_PASSWORD=quackgis ./container/test-compat.sh
PG_PORT=55432 PG_PASSWORD=quackgis ./container/run-postgis-fixtures.sh
PG_HOST=127.0.0.1 PG_PORT=55432 python3 container/tests/test_psycopg.py
```

## Engine vs facade separation

**Engine tests** (`ci/all-checks.sh` without `--facade`):
- `cargo test --lib` — Rust unit tests for FFI safety and kernel correctness.
- `tests/run_sql.sh` — SQL regression suite run against DuckDB directly.
- `ci/package-and-smoke.sh` — extension packaging + backend smoke.
- `benchmarks/scale_harness.sh` — DuckLake pruning evidence.

**Facade tests** (`ci/all-checks.sh --facade`):
- Container smoke test (6 checks).
- PostGIS compatibility suite (20 checks).
- PostGIS fixture suite (PG-syntax SQL through the facade).
- DuckLake storage + persistence (5 phases).
- psycopg client suite (prepared statements, params, metadata).

Engine tests verify the spatial kernel. Facade tests verify client-facing
behavior. Both are required for release.
