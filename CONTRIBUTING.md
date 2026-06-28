# Contributing to duckdb_sedona

This guide tells you how to add a spatial function, which backend to choose,
and what tests/docs are required before it ships.

## Quick start

```sh
# Build
export PKG_CONFIG_PATH=/var/home/linuxbrew/.linuxbrew/lib/pkgconfig
export LIBCLANG_PATH=/var/home/linuxbrew/.linuxbrew/Cellar/llvm/22.1.8/lib
export LD_LIBRARY_PATH=/var/home/linuxbrew/.linuxbrew/lib
cargo build --release

# Package
./target/release/sedonadb-package target/release/libsedonadb.so \
    build/dev/sedonadb.duckdb_extension linux_amd64

# Test
cargo test --lib
./tests/run_sql.sh
./ci/package-and-smoke.sh
python3 tools/catalog_audit.py --check
```

## Architecture

The extension uses a **declarative macro dispatch** architecture:

1. **`src/registry.rs`** — one line per function. Pick a macro, add the SQL
   name and the Rust function path. The macro generates the FFI callback.
2. **`src/dispatch.rs`** — generic vectorized executors. Each reads WKB blobs
   from DuckDB chunks, applies the operation, writes results back.
3. **`src/functions.rs`** — local `geo`-crate algorithms.
4. **`src/bridge.rs`** — DuckDB-chunk ⇄ Arrow bridge to literal SedonaDB kernels.
5. **`src/geos_backend.rs`** — narrow WKB-in/WKB-out boundary to GEOS.
6. **`src/dump.rs`** — set-returning table functions.
7. **`src/raster.rs`** — GDAL raster table functions and `ST_Value`.

## How to add a new function

### Step 1: Choose a backend

Use this decision tree:

| Condition | Backend | Where |
|-----------|---------|-------|
| SedonaDB has a matching kernel with the same signature | **Literal SedonaDB bridge** | `register_sedona_*!` macro in `registry.rs` |
| Function needs planar topology, overlay, or make-valid | **GEOS** | `src/geos_backend.rs` |
| Function is a standard OGC accessor/predicate/measure | **Local `geo` crate** | `src/functions.rs` |
| Function does CRS reprojection | **PROJ** | `src/functions.rs` + `register_geom_int2_to_geom!` |
| Function reads/writes raster data | **GDAL** | `src/raster.rs` |
| Function is set-returning (dump, rings) | **Local Rust** | `src/dump.rs` |

**Prefer the literal SedonaDB bridge** wherever a matching kernel exists. This
makes the extension a true SedonaDB superset and avoids maintaining duplicate
implementations.

### Step 2: Implement

**Local geo function:**
```rust
// src/functions.rs
pub fn my_function(g: &Geom) -> Option<Geom> {
    // ... algorithm using geo crate ...
}
```

**Literal SedonaDB bridge:**
```rust
// src/registry.rs — one line, no new code:
register_sedona_blob_blob!("st_myfunc", "st_myfunc");
```

**GEOS function:**
```rust
// src/geos_backend.rs
pub fn my_topology_op(wkb: &[u8]) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let result = g.some_geos_op().ok()?;
    to_wkb(&result)
}
```

### Step 3: Register

Add one line to `src/registry.rs`:

```rust
// Local:
register_unary_geom!("st_myfunc", functions::my_function);

// Bridge:
register_sedona_blob_blob!("st_myfunc", "st_myfunc");

// Predicate:
register_predicate!("st_myfunc", functions::my_predicate);
```

### Step 4: Test

1. Add a SQL regression check to the appropriate `tests/reference/monthN_fixtures.sql`
   or create a new fixture file.
2. If the function has a literal SedonaDB twin, add a parity check:
   `st_myfunc(x) == sedona_st_myfunc(x)`.
3. Test NULL propagation and at least one edge case (empty geometry, invalid
   input, out-of-bounds index).
4. Run `./tests/run_sql.sh` and verify PASS/FAIL counts.
5. Run `python3 tools/catalog_audit.py --check` to catch doc drift.

### Step 5: Document

1. Update `COMPATIBILITY.md` with the function's status (✅ / ⚠️ / 🔄).
2. Note any semantic delta from PostGIS.
3. If the function is routed to a literal kernel, add a routing note.
4. Update counts in README.md, ROADMAP.md, COMPATIBILITY.md (run the drift check).

## Backend decision guide

### When to use the literal SedonaDB bridge

Use it when:
- SedonaDB has a matching kernel with compatible argument types.
- The function is a simple accessor, constructor, or transform.
- You want the same behavior SedonaDB itself produces.

The bridge adds a small per-call overhead (DuckDB-chunk ⇄ Arrow conversion)
but guarantees SedonaDB-identical results. Always add a parity fixture.

### When to use GEOS

Use it when:
- The function needs robust planar topology (Node, Polygonize, BuildArea).
- The function needs overlay operations that the local `geo` crate cannot
  handle on invalid/complex input.
- You need PostGIS-grade fidelity for hard geometry.

GEOS functions live in `src/geos_backend.rs` and operate on raw WKB bytes to
avoid double conversion through `geo_types`. They fail closed to `NULL`.

### When to use the local `geo` crate

Use it when:
- SedonaDB lacks the function (PostGIS extras like ConcaveHull, Hausdorff).
- The function is a DuckDB-specific shape (table function, aggregate).
- You need a fast path for valid geometry with GEOS fallback for edge cases.

Guard relate-based predicates and boolean ops with `ensure_valid` to avoid
crashes on invalid real-world polygons.

### When to use GDAL/PROJ

- **PROJ** for CRS reprojection (`ST_Transform`).
- **GDAL** for raster I/O (`ST_PixelData`, `ST_Value`, `ST_RasterInfo`).

Both are linked statically; runtime library paths must be set for the host
platform (Linuxbrew on this dev machine).

## Semantic deltas

Every deviation from PostGIS behavior must be documented in
`COMPATIBILITY.md` under the "Compatibility debt log" section, with:

1. The function name(s) affected.
2. What PostGIS does vs what we do.
3. A fixture that demonstrates the delta (or an explicit defer rationale).

## Definition of done

A new function is done when:

1. ✅ Namespace matches PostGIS/SedonaDB where feasible.
2. ✅ Canonical backend chosen and documented.
3. ✅ Invalid/unsupported inputs return `NULL` or a documented error; no panic.
4. ✅ SQL regression covers normal behavior + at least one edge case.
5. ✅ If overlapping SedonaDB exists, parity fixture added or divergence documented.
6. ✅ README/ROADMAP/COMPATIBILITY counts updated (drift check passes).
7. ✅ `cargo test --lib`, release build, SQL suites, and smoke test pass.
