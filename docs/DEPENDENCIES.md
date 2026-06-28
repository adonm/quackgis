# Dependency matrix and load diagnostics

## Supported dependency versions

| Dependency | Minimum | Tested | Source | Purpose |
|---|---|---|---|---|
| DuckDB | 1.5.x | 1.5.4 | system / download | host engine / loadable-extension ABI |
| GEOS | 3.8.0 | 3.14.1 | Linuxbrew / system | planar topology, overlay fallback, ST_Relate |
| GDAL | 3.5 | 3.13.1 | Linuxbrew / system | raster I/O (st_pixeldata, st_raster_info, st_value) |
| PROJ | 9.0 | 9.4+ | Linuxbrew / system | CRS transforms (ST_Transform) |
| LLVM/libclang | 14 | 22.1.8 | Linuxbrew / system | bindgen for GDAL/geos-sys at build time |
| DuckLake | built-in | built-in | DuckDB extension | spatial lakehouse layout (optional) |

## Build environment setup

### Linux (Linuxbrew)

```bash
# Install GDAL, GEOS, PROJ, and LLVM via Linuxbrew
brew install gdal geos proj llvm

# Set build environment
export PKG_CONFIG_PATH=/var/home/linuxbrew/.linuxbrew/lib/pkgconfig
export LIBCLANG_PATH=/var/home/linuxbrew/.linuxbrew/Cellar/llvm/22.1.8/lib
export LD_LIBRARY_PATH=/var/home/linuxbrew/.linuxbrew/lib

# Build and package
cargo build --release
./target/release/sedonadb-package \
    target/release/libsedonadb.so \
    build/dev/sedonadb.duckdb_extension \
    linux_amd64
```

### Container (reproducible)

```bash
./ci/container-build.sh    # builds the GDAL/Rust builder image
./ci/container-test.sh     # cargo test --lib + cargo build --release inside
```

## Runtime load diagnostics

If `LOAD sedonadb;` fails, the error usually points to a missing shared library:

| Error pattern | Missing library | Fix |
|---|---|---|
| `libgeos_c.so: cannot open shared object file` | GEOS | `export LD_LIBRARY_PATH="$(brew --prefix geos)/lib:$LD_LIBRARY_PATH"` |
| `libgdal.so: cannot open shared object file` | GDAL | `export LD_LIBRARY_PATH="$(brew --prefix gdal)/lib:$LD_LIBRARY_PATH"` |
| `libproj.so: cannot open shared object file` | PROJ | `export LD_LIBRARY_PATH="$(brew --prefix proj)/lib:$LD_LIBRARY_PATH"` |
| `Extension not found` | wrong path | Use full path: `LOAD '/path/to/sedonadb.duckdb_extension';` |
| `symbol lookup error: undefined symbol: GEOSRelate_r` | GEOS too old | upgrade to GEOS ≥ 3.8.0 |

### Verifying the extension loads

```bash
duckdb -unsigned -cmd "LOAD 'build/dev/sedonadb.duckdb_extension';" \
  -c "SELECT st_geometrytype(st_geomfromtext('POINT(0 0)'));"
# Expected: ST_Point
```

### Verifying individual backends

```bash
./ci/package-and-smoke.sh
# Runs 18 smoke checks: local, SedonaDB bridge, aggregates, GEOS, spheroid,
# raster, PROJ, table functions, overlay, DumpRings, ContainsProperly,
# routing parity, ST_Relate, ST_AsSVG, relate-pattern.
```

## CI pipeline

The unified CI script (`ci/all-checks.sh`) runs every quality gate:

1. **Rust unit tests** — `cargo test --lib`
2. **Drift gate** — catalog check + compat check + ledger freshness
3. **SQL suite** — all regression fixtures + DuckLake tests (error-sensitive)
4. **Package + smoke** — build, package, load in DuckDB, verify all backends
5. **Scale harness** — DuckLake layout comparison at smoke tier

```bash
./ci/all-checks.sh
```

All five phases must pass. The script exits non-zero on any failure.

## Extension packaging

The `.duckdb_extension` file is a `.so` with a 512-byte DuckDB trailer appended
by the `sedonadb-package` binary. The trailer encodes the platform name and a
checksum that DuckDB validates at load time.

```bash
# Build the packager (one-time)
cargo build --release

# Package
./target/release/sedonadb-package \
    target/release/libsedonadb.so \
    build/dev/sedonadb.duckdb_extension \
    linux_amd64    # or darwin_amd64 / darwin_arm64
```

## Known platform issues

- **macOS**: GDAL/GEOS/PROJ via Homebrew work at build time. CI runs on
  `macos-14` (Apple Silicon) via `.github/workflows/ci.yml`. The `darwin_amd64`
  / `darwin_arm64` platform strings are supported by the packager.
- **GEOS version mismatch**: The `geos` Rust crate requires GEOS ≥ 3.8.0 at
  runtime. If the system GEOS is older, `ST_MakeValid`, `ST_Node`,
  `ST_Polygonize`, `ST_Relate`, and the overlay fallback will fail closed
  (return NULL).
- **GDAL bindgen**: The `gdal` Rust crate uses bindgen at build time, which
  requires `libclang.so`. If `LIBCLANG_PATH` is not set, the build fails with
  a clear error pointing to the missing library.

## Platform matrix

| Platform | Build | CI | Runtime deps |
|---|---|---|---|
| linux_amd64 | ✅ | ✅ `ubuntu-22.04` | libgdal ≥ 3.5, libgeos_c ≥ 3.8, libproj ≥ 9.0 |
| darwin_arm64 | ✅ | ✅ `macos-14` | Homebrew GDAL/GEOS/PROJ |
| darwin_amd64 | ✅ | manual | Homebrew GDAL/GEOS/PROJ |
| windows | ⏳ | — | — |

See [docs/RELEASE_CHECKLIST.md](./RELEASE_CHECKLIST.md) for the full release
packaging process.

## GitHub Actions CI

The workflow at `.github/workflows/ci.yml` runs on every push/PR:

| Job | Runner | Scope |
|---|---|---|
| `linux-amd64` | `ubuntu-22.04` | Full 5-phase pipeline (`ci/all-checks.sh`) |
| `macos-arm64` | `macos-14` | Build + Rust tests + smoke test |
| `lint` | `ubuntu-22.04` | `cargo fmt --check` + `cargo clippy` (non-blocking) |
