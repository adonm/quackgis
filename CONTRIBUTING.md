# Contributing

Two tracks: **engine** (Rust DuckDB extension) and **facade** (container, SQL
stubs, tests, deployment).

## Engine

```sh
cargo test --lib
./tests/run_sql.sh
./ci/package-and-smoke.sh
python3 tools/catalog_audit.py --check
```

Architecture: declarative macro dispatch. One registry line per function in
`src/registry.rs`. One generic executor per result shape in `src/dispatch.rs`.
WKB parse/validate in `src/geometry.rs` (trust boundary). See the module
headers in `src/` for details.

Adding a function: pick the backend (literal SedonaDB bridge, GEOS, local
GeoRust, PROJ, GDAL), register one line in `registry.rs`, add tests, update
`COMPATIBILITY.md` if user-visible.

## Facade

```sh
./container/build.sh
./container/smoke-test.sh
./container/run-all-tests.sh --no-build
```

Init scripts in `container/init.d/` define the PG-level surface: geometry
DOMAIN, operators, function stubs, aggregates, layout helpers. The bridge table
pattern (`quackgis._bridge`) routes standalone spatial calls to DuckDB.

`container/generate-stubs.sh` reads `src/registry.rs` and generates stubs for
all `st_*` functions not manually defined. **Known bug**: generated stubs have
wrong arities — needs fixing to read dispatch macro signatures.

## Rules

- No silent geometry semantic changes.
- Validate at trust boundaries and fail closed.
- Rust-first owned code.
- Keep docs short — see [ROADMAP.md](./ROADMAP.md) for current priorities.
