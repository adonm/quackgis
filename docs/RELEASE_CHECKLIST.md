# Release checklist

Steps to cut a new sedonadb extension release.

## 1. Verify gates

```bash
./ci/all-checks.sh           # must pass all 5 phases
python3 tools/catalog_audit.py --check
python3 tools/catalog_audit.py --compat-check
python3 tools/catalog_audit.py --generate-ledger   # must be up to date
python3 tools/catalog_audit.py --export-json        # must be up to date
```

All must pass. Fix any drift before proceeding.

## 2. Update docs

- [ ] CHANGELOG.md — add release section with milestone summary, catalog counts,
      delta count, verification commands.
- [ ] ROADMAP.md — mark milestone(s) as ✅ LANDED.
- [ ] COMPATIBILITY.md — ensure delta log and backlog reflect current state.
- [ ] README.md — ensure catalog counts and roadmap pointer are current.
- [ ] docs/DEPENDENCIES.md — update tested versions if deps changed.

## 3. Build and package artifacts

For each supported platform:

```bash
# Linux amd64
cargo build --release
./target/release/sedonadb-package \
    target/release/libsedonadb.so \
    build/dev/sedonadb.duckdb_extension \
    linux_amd64

# macOS arm64 (Apple Silicon)
cargo build --release
./target/release/sedonadb-package \
    target/release/libsedonadb.dylib \
    build/dev/sedonadb.duckdb_extension \
    darwin_arm64

# macOS amd64 (Intel)
cargo build --release
./target/release/sedonadb-package \
    target/release/libsedonadb.dylib \
    build/dev/sedonadb.duckdb_extension \
    darwin_amd64
```

## 4. Smoke-test each artifact

```bash
LD_LIBRARY_PATH="$(brew --prefix gdal)/lib" duckdb -unsigned \
    -cmd "LOAD 'build/dev/sedonadb.duckdb_extension';" \
    -c "SELECT st_geometrytype(st_geomfromtext('POINT(0 0)'));"
# Expected: ST_Point
```

Or run the full smoke suite:

```bash
./ci/package-and-smoke.sh    # 18 backend checks
```

## 5. Checksums

```bash
sha256sum build/dev/sedonadb.duckdb_extension > build/dev/sedonadb.duckdb_extension.sha256
```

## 6. Tag and release

```bash
git tag -a v2.x.0 -m "Release v2.x.0"
git push origin v2.x.0
```

Create a GitHub Release with:
- Release notes from CHANGELOG.md
- Artifact files (.duckdb_extension + .sha256)
- Supported DuckDB version
- Dependency version matrix

## Platform matrix

| Platform | Status | Build deps | Runtime deps |
|---|---|---|---|
| linux_amd64 | ✅ CI-tested | GDAL, GEOS, PROJ, libclang | libgdal, libgeos_c, libproj |
| darwin_arm64 | ✅ Buildable | Homebrew GDAL/GEOS/PROJ/LLVM | same (Homebrew) |
| darwin_amd64 | ✅ Buildable | Homebrew GDAL/GEOS/PROJ/LLVM | same (Homebrew) |
| windows | ⏳ Not tested | — | — |

## DuckDB ABI compatibility

The extension is built against `libduckdb-sys` with the `loadable-extension`
feature. DuckDB supplies the C-API symbols at load time. The extension must
match the DuckDB ABI version:

| libduckdb-sys | DuckDB CLI |
|---|---|
| 1.10504.0 | 1.5.4 |
