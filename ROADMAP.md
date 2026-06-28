# Roadmap to a highest-fidelity PostGIS + SedonaDB stack for DuckDB

Status of the `sedonadb` DuckDB extension against the PostGIS and Apache
SedonaDB spatial surfaces. The v2.0 target is reached (Milestone 20); this
document records the target definition, the landed milestone evidence, and the
post-target plan that keeps the extension focused, usable, and maintainable.

## North-star target

Two first-class product goals (see [ARCHITECTURE.md](./ARCHITECTURE.md) for the
full design):

1. **PostGIS workload portability.** Existing PostGIS analysis SQL should port
   with mostly mechanical rewrites: familiar `ST_*` names, argument order, units,
   NULL behavior, and edge-case semantics, plus documented rewrites for the
   PostgreSQL-isms DuckDB cannot host (`&&`, `<->`, GiST, typmods).
2. **DuckDB/DuckLake-native spatial lakehouse.** Spatial data should scale in
   DuckLake and Hive-partitioned stores the way Sedona scales in Spark: through
   deterministic, materialized spatial partition keys, bbox zone-map columns,
   space-filling-curve clustering, and explicit cell-covering query patterns —
   not hidden planner hooks.

This target (stated in README) is demand-backed, not an aspirational
catalog-inflation exercise. The primary compatibility audience is the common
**PostGIS + SedonaDB** stack: PostGIS SQL/workload portability first, literal
SedonaDB kernel fidelity wherever kernels exist, and DuckLake spatial lakehouse
operations as the scale layer.

The target remains a practical **PostGIS-compatible analysis surface plus a
literal SedonaDB bridge**, and stays maintainable through:

1. **PostGIS-compatible SQL surface.** Prefer familiar `ST_*` names, arities,
   argument order, units, NULL behavior, and edge-case semantics. Where DuckDB,
   SedonaDB, Rust libraries, or missing infrastructure make exact PostGIS
   behavior impossible, document the mismatch and test it explicitly.
2. **SedonaDB superset, literal by default.** Every Apache SedonaDB SQL kernel
   that can be bridged safely should be callable in DuckDB and should be the
   canonical implementation for matching `st_*` functions. The literal bridge is
   not just an oracle; it is the preferred engine wherever SedonaDB already has a
   function.
3. **Highest fidelity over catalog inflation.** Never ship a function that can
   silently return wrong geometry. Prefer `NULL`, a documented limitation, or an
   unimplemented item over approximate behavior that looks authoritative.
4. **DuckDB-native usability.** Spatial workflows should feel natural in DuckDB:
   vectorized chunk execution, WKB interop with DuckDB `spatial`, table functions
   for set-returning/raster/join workflows, and SQL as the composition language.
5. **Maintainable growth.** New capability should expand shared executor families
   or narrow backend boundaries (Sedona bridge, GEOS, GDAL, PROJ, GeographicLib),
   not add one-off semantic snowflakes.
6. **Explicit layout over hidden magic.** Storage-layout helpers (partition
   keys, curve sort keys, covering cells) are plain deterministic functions over
   geometry. DuckLake's catalog stays the source of truth; the extension never
   maintains its own authoritative index state.

## SQL namespace policy

- `st_*`: the user-facing namespace. Match PostGIS/SedonaDB names, arities,
  argument order, units, and NULL behavior wherever feasible.
- `sedona_st_*`: explicit literal Apache SedonaDB bridge functions. These are
  useful to users, tests, migration/debugging, and fidelity comparisons.
- Extension-specific helpers (`sedona_join`, `*_crs`, benchmarks/tools) are
  allowed when DuckDB needs a different shape than PostGIS/SedonaDB, but should be
  documented as DuckDB-native workflow helpers rather than compatibility claims.

Naming policy:

- Product target wording should be **PostGIS + SedonaDB for DuckDB**. PostGIS is
  named first because it is the most common SQL stack users are likely migrating
  from; SedonaDB remains first-class as the literal kernel bridge and scale
  design inspiration.
- Package/extension name stays `sedonadb` for continuity and because the literal
  Apache SedonaDB bridge is a core differentiator.
- Migration/test tooling should use `sedonadb-*` names (`sedonadb-rewrite`,
  `sedonadb-upstream-tests`) even when the source dialect is PostGIS.

Implementation policy:

- Do **not** maintain two independent implementations for the same semantics as a
  permanent state. Once a SedonaDB bridge kernel is supported and validated,
  prefer routing the public `st_*` function to it.
- Keep local Rust implementations for: functions SedonaDB lacks, PostGIS-compat
  capabilities beyond SedonaDB, DuckDB-specific table/aggregate shapes, bridge
  unsupported types, or temporary fallback/performance experiments.
- If a local implementation overlaps SedonaDB, either deprecate it internally,
  convert it into a thin wrapper over the bridge, or document why it intentionally
  diverges.
- Backend choices should be boring and canonical: Apache SedonaDB for SedonaDB
  kernels, GEOS for planar topology, GeographicLib/Karney for spheroid geodesics,
  PROJ for CRS transforms, GDAL for raster I/O, and DuckDB SQL for relational/map
  algebra.

## Quality gates for every new capability

- **Fidelity first:** add local-vs-literal SedonaDB tests when a bridged kernel
  exists; add PostGIS/SedonaDB reference fixtures for hard edge cases otherwise.
- **Fail closed:** invalid WKB, unsupported encodings, missing bridge functions,
  and undefined operations must not panic or fabricate geometry.
- **Vectorized and packageable:** every feature must work through DuckDB chunks,
  SQL regressions, release packaging, and the smoke test.
- **Maintainable by design:** prefer one registry line plus a shared executor;
  keep FFI, Arrow, GDAL/PROJ, GEOS, and topology code isolated behind small
  boundaries.
- **Usable docs:** every non-obvious semantic difference, runtime dependency, and
  extension-specific helper needs a short README/ROADMAP note and at least one SQL
  example or regression.

## Target status

As of v2.0.0 / Milestone 20, the north-star target is met for the supported
scope. The project is no longer in a "close the gap" phase; future milestones
must keep the gates green, retire documented deltas when there is a canonical
path, and expand breadth only with fixtures and fail-closed behavior.

The extension already has a broad vector/geography/raster surface over WKB BLOBs:

- Constructors and I/O: WKT/WKB/EWKT/EWKB, typed WKT/WKB constructors, point/Z/M
  constructors, GeoJSON/HexEWKB/SVG/KML/TWKB/MVT output.
- Accessors and predicates: dimension, points/geometries/rings, XY/ZM accessors,
  bbox accessors, DE-9IM predicates including `ST_Relate` matrix/pattern via
  GEOS, validity checks, ordering equality.
- Measurements and processing: area/length/distance/perimeter, Hausdorff/Frechet,
  max/longest/shortest line, affine/editing transforms, simplify/segmentize,
  hulls/oriented envelope, triangulation, make-valid, minimum clearance/circle.
- Aggregates and set-returning functions: collect/union/envelope/makeline/
  intersection aggregates plus `ST_Dump`, `ST_DumpPoints`, `ST_DumpSegments`,
  `ST_DumpRings`, `ST_IsValidDetail`, and `ST_CoveringQuadKeys`.
- CRS/geography: `ST_Transform`, sphere geodesics, WGS84 spheroid geodesics.
- Hard algorithms: GEOS-backed `ST_Node`, `ST_Polygonize`, `ST_BuildArea`,
  `ST_VoronoiPolygons`, `ST_Snap`, `ST_MakeValid`, and `ST_Relate`.
- Raster: `st_raster_info`, `st_raster_stats`, `st_raster_transform`,
  `st_pixeldata(path, band)` for DuckDB-native map algebra, and `st_value` for
  point sampling.
- Spatial layout (L3): `ST_QuadKey`/`ST_GeoHash` cell keys, `ST_Hilbert`/
  `ST_Morton` sort keys, `ST_TileEnvelope`, `ST_BBoxIntersects`, covering-cell
  table function, and partition sizing helpers.
- Migration tooling: `tools/postgis_rewriter.py` linter/rewriter,
  `tools/import_upstream_tests.py` external corpus importer, migration workbook,
  generated ledger + JSON compatibility export.

**Literal Apache SedonaDB is linked.** `src/bridge.rs` invokes the real
`sedona-functions` DataFusion UDF kernels directly from DuckDB via a
DuckDB-chunk ⇄ Arrow bridge. 72 `sedona_st_*` functions are registered and
runtime-verified, including CRS-tagged returns, CRS sidecar extractors,
WKT/WKB typed constructors, Z/M point constructors, and constant-scalar argument
detection. 48 public `st_*` functions already route to the literal SedonaDB
kernel.

Current verification baseline:

- Rust unit tests: 75 pass.
- SQL regressions: 735 pass / 0 fail (703 standard + 19 DuckLake + 13 macros).
- Release smoke test: 18 backend checks pass (local, SedonaDB, aggregates,
  GEOS, spheroid, raster, PROJ, table functions, overlay, DumpRings,
  ContainsProperly, routing parity, `ST_Relate`, `ST_AsSVG`).
- Catalog: 254 registered SQL functions (180 `st_*` public + 72 `sedona_st_*` bridge + 2 extension-specific). 46 public `st_*` routed to literal kernel. Audit with `python3 tools/catalog_audit.py`.

## Capability matrix (category-level)

Legend: ✅ shipped · 🟡 partial · ⏳ not yet · ➖ intentionally out of scope.

| Category | PostGIS | SedonaDB | sedonadb extension | Notes |
|---|---|---|---|---|
| Constructors (WKT/WKB/EWKT/EWKB, typed `*FromText`) | ✅ | ✅ | ✅ | WKT/WKB/EWKT/EWKB + typed constructors + point/Z/M constructors. |
| Output (`ST_AsText/Binary/EWKB/GeoJSON/HexEWKB/SVG/KML/TWKB/MVT`) | ✅ | ✅ | ✅ | All output encoders shipped (SVG M16, KML/TWKB M23, MVT M23b). |
| Accessors (X/Y/Z/M, dims, rings, N-th geometry/point) | ✅ | ✅ | 🟡 | Broad 2D + bridged Z/M accessors; full Z/M-preserving local pipeline remains limited. |
| DE-9IM predicates (`Intersects`…`Covers`, `Relate`, `OrderingEquals`) | ✅ | ✅ | ✅ | Guarded for invalid input; `ST_Relate` via GEOS; `ST_Contains`/`ST_Within` match PostGIS DE-9IM exactly (M22: boundary delta retired). |
| Measurements (`Area/Length/Distance/Perimeter/Azimuth/Hausdorff/...`) | ✅ | ✅ | ✅ | Core, distance-family, clearance-family shipped. |
| Boolean set ops (`Union/Intersection/Difference/SymDiff`) | ✅ | 🟡 | ✅ | Scalar polygonal set ops and intersection aggregate shipped. |
| `ST_MakeValid` / validity | ✅ | 🟡 | ✅ | Robustness hardening shipped. |
| Editing (`Translate/Scale/Rotate/Flip/Reverse/Affine/Segmentize/...`) | ✅ | ✅ | ✅ | Includes 6-param `ST_Affine`; `ST_Snap` via GEOS. |
| Geometry processing (`Buffer/Simplify/Hulls/Triangulate/Voronoi`) | ✅ | 🟡 | ✅ | Bounded Voronoi polygons via GEOS. |
| Topology editing (`Node/Polygonize/BuildArea`) | ✅ | 🟡 | ✅ | GEOS-backed. |
| Linear referencing (`LineInterpolatePoint/Locate/Substring`) | ✅ | 🟡 | ✅ | Done. |
| Aggregates (`Collect/Union/Envelope/Intersection/MakeLine`) | ✅ | ✅ | ✅ | Collect/Union/Envelope/MakeLine/Intersection aggregate family shipped. |
| Geography/geodesic ops | ✅ | ✅ | ✅ | Sphere + WGS84 spheroid (`Distance/DWithin/Length/Area`) done; custom spheroid parameter open. |
| CRS / PROJ (`ST_Transform`, SRID) | ✅ | ✅ | ✅ | `ST_Transform` via PROJ; SRID represented at extension-native fidelity with CRS sidecars where needed. |
| Spatial index join (`&&`, GiST/R-tree workflows) | ✅ | ✅ | ✅ | `sedona_join` table fn over spilled parquet + bbox prefilter helpers. |
| Raster / map algebra | ✅ | ✅ | 🟡 | Info/stats/transform/pixel streaming/value sampling done; clip is a tested SQL workflow; `ST_AsRaster` in active backlog (M23). |
| 3D / Z-M geometry + SFCGAL surfaces | ✅ | ⏳ | ⏳ | Z/M bridge surface exists; full 3D solid/surface operations are out of scope until mature Rust SFCGAL/CGAL exists. |
| Topology schema / Tiger geocoder / address standardizer | ✅ | ➖ | ➖ | PostgreSQL-specific/niche subsystems; intentionally out of scope. |

## What "PostGIS + SedonaDB compatibility" realistically means

A 100% byte-compatible PostGIS clone is not the target. PostgreSQL operators,
GiST planner integration, SFCGAL 3D solids, topology schemas, Tiger/geocoder, and
some raster administration APIs do not map cleanly to a DuckDB loadable
extension.

The target is higher-value and more focused:

1. **SedonaDB-plus:** every practical SedonaDB vector SQL function available in
   DuckDB, with the literal bridge as the canonical implementation for matching
   public `st_*` functions wherever signatures and return types fit.
2. **PostGIS-compatible core:** the common PostGIS vector/geography/CRS/raster
   analysis surface under familiar `ST_*` names, with exact semantics where
   feasible and tested/documented deltas where not.
3. **DuckDB-native workflows:** install/load/package cleanly, operate on WKB BLOBs
   that interoperate with DuckDB `spatial`, stream through vectorized chunks, and
   provide join/raster workflows that fit DuckDB rather than PostgreSQL internals.
4. **Maintainable growth:** new functions should expand a small number of shared
   executor families and reference tests, not create per-function FFI or semantic
   snowflakes.

## Completed milestone cycle (Milestones 1–3)

This cycle established the compatibility contract, filled high-value fidelity
gaps, and made the extension usable enough for real DuckDB workflows.

### Milestone 1 — compatibility contract and namespace polish — ✅ LANDED

Outcome: users can see exactly what is compatible, what is bridged, and what
differs before they port SQL.

Landed:

1. **Generated catalog audit.** `tools/catalog_audit.py` reads `src/registry.rs`
   and emits the registered SQL catalog grouped by provenance: literal SedonaDB,
   local geo, GEOS, PROJ, GDAL/raster, aggregates, and table functions. Run with
   `python3 tools/catalog_audit.py [--markdown]`.
2. **PostGIS/SedonaDB compatibility table.** `COMPATIBILITY.md` lists common
   PostGIS functions and their status: supported, alias, semantic delta, not yet,
   or intentionally out of scope.
3. **Namespace cleanup.** Added `ST_Force3D`/`3DZ`/`3DM`/`4D` routed to the
   literal SedonaDB kernel (Z/M dimension forcing). Documented semantic delta:
   explicit z/m parameter required (PostGIS defaults to 0).
4. **Literal routing pass.** 36 public `st_*` functions now route to the literal
   SedonaDB kernel (up from 32).
5. **Reference fixtures expansion.** `tests/reference/m1_fixtures.sql` adds
   22 checks: invalid geometry (bowtie), empty/NULL propagation, antimeridian
   geography, CRS round-trip stability, GEOS snap degeneracy, Voronoi single
   point, force-dimension family, large coordinates, nested collections, polygon
   holes, degenerate lines.

### Milestone 2 — high-fidelity capability work — ✅ LANDED

Outcome: fill high-value gaps with canonical engines, especially where incorrect
approximations would be harmful.

Landed:

1. **ST_Subdivide audit.** Existing local implementation verified: grid-clip
   polygon subdivision using `geo::BooleanOps::intersection`, line chunking,
   collection flattening. Edge-case fixtures added: point passthrough, line
   split, polygon subdivision, empty geometry.
2. **ST_IntersectionAgg hardening.** Existing cascaded intersection aggregate
   verified with adversarial fixtures: overlapping polygons, disjoint → NULL
   (PostGIS semantics), empty table → NULL, three-polygon cascade validity.
3. **ST_Value point sampling.** New scalar function
   `st_value(path, band, x, y) → DOUBLE` via GDAL. Inverts the GeoTransform to
   convert geographic coordinates to pixel space, reads one pixel. Returns NULL
   for out-of-bounds/nodata. Documented: for bulk sampling, use `st_pixeldata`.
4. **Raster clipping workflow.** DuckDB-native recipe documented and tested:
   join `st_pixeldata` against `st_raster_transform` to compute geographic
   coordinates per pixel, filter by bounding box or geometry predicate. No
   custom ST_Clip needed — SQL is the clip engine.
5. **Spheroid parameter fidelity.** Evaluated and deferred: WGS84-only
   (Karney/GeographicLib) remains the default. Custom spheroid parsing would
   add complexity without clear user demand. Antipodal stability verified.

### Milestone 3 — usability, scale, and release readiness — ✅ LANDED

Outcome: the full-capability extension is easy to install, debug, and use for
real DuckDB spatial analytics.

Landed:

1. **Spatial join ergonomics.** Documented the canonical bbox-prefilter + exact
   predicate pattern in `EXAMPLES.md` with copy-pasteable SQL. Tested the
   pattern with a 3-polygon × 2-polygon join that verifies bbox candidates and
   exact intersections agree.
2. **Performance budgets.** New `benchmarks/perf_budget.sql` covers 7 backend
   families: bridge overhead (100k), GEOS topology (10k), spheroid geodesics
   (100k), raster scan + ST_Value, local geo pipeline (100k), aggregates (100k),
   and table functions (100k). Each section prints wall-clock timing.
3. **Release packaging.** Smoke test expanded from 7 to 11 backend checks: local
   geo, SedonaDB, aggregate-envelope, aggregate-collect, GEOS, spheroid,
   raster-pixeldata, raster-value, PROJ transform, and table function (ST_Dump).
4. **User-facing examples.** New `EXAMPLES.md` with 11 copy-pasteable workflows:
   quick start, GeoParquet ingest, CRS transform join, geodesic distance,
   dissolve, dump, raster point sampling, raster reclassification, raster clip,
   sedona_join R-tree, and topology workflows.
5. **Maintenance cleanup.** Dormant local functions in `functions.rs` annotated
   with `DORMANT:` doc comments — the public `st_*` routes to the literal
   SedonaDB kernel. Added a contributor note explaining the routing convention.
   Milestone 3 fixtures verify routed `st_*` produces identical results to
   `sedona_st_*` across the corpus.

## Completed milestone cycle (Milestones 4–6)

This cycle stayed deliberately focused on **higher fidelity, clearer SQL
compatibility, better usability, and lower maintenance cost**. Raw catalog count
was not treated as a success metric unless the new functions were backed by
canonical engines, reference tests, and documented semantics.

Planning rules:

- Ship small vertical slices: registry entry, backend/executor shape if needed,
  reference tests, docs, benchmark/smoke coverage when relevant.
- Prefer canonical backends over local algorithms: literal SedonaDB for matching
  kernels, GEOS for planar topology and overlay, GeographicLib/Karney for
  spheroids, PROJ for CRS, GDAL for raster I/O, DuckDB SQL for relational/raster
  algebra.
- Treat SQL namespace compatibility as a product feature: familiar `ST_*` names,
  PostGIS/SedonaDB argument order, explicit overload decisions, and no stale
  catalog counts.
- Do not ship approximate geometry that looks authoritative. Unsupported inputs
  should return `NULL`, produce a documented error, or remain unimplemented.
- Use Apache SpatialBench as a bounded workload/performance/robustness QA
  harness, not as the primary correctness oracle. Correctness stays in focused
  SQL fixtures; SpatialBench proves realistic joins/scans finish, return stable
  row counts, and stay within broad performance budgets.
- Defer systems that would blur the product: PostgreSQL planner hooks, topology
  schema, Tiger/geocoder, address standardizer, SFCGAL solids before mature Rust
  bindings, and a custom raster expression language.

### Milestone 4 — fidelity harness and SQL namespace convergence — ✅ LANDED

Outcome: every important compatibility claim is backed by deterministic evidence,
and users can port more SedonaDB/PostGIS SQL without guessing.

Landed:

1. **Catalog drift check.** `tools/catalog_audit.py --check` reads committed doc
   counts from README/ROADMAP/COMPATIBILITY and compares to the live registry,
   exiting non-zero on drift. Prevents stale counts from accumulating.
2. **Literal routing expansion.** 11 more public `st_*` functions routed to the
   literal SedonaDB kernel (up from 36 to 47): `ST_Point`, `ST_MakePoint`,
   `ST_MakeLine`, `ST_Azimuth`, `ST_Affine`, `ST_Rotate`, `ST_Translate`,
   `ST_Scale`, `ST_LineFromText`, `ST_PointFromText`, `ST_PolygonFromText`.
   Typed WKT constructors now validate geometry type (PostGIS fidelity:
   mismatched WKT returns NULL instead of silently parsing).
3. **Namespace polish.** Added `ST_Polygon(linestring, srid)` — PostGIS
   polygon constructor. Updated COMPATIBILITY.md with routing notes for all
   newly routed functions.
4. **Differential reference harness.** `tests/reference/m4_fixtures.sql`
   with 36 checks: parity `st_* == sedona_st_*` for all routed functions,
   ST_Polygon edge cases, typed-constructor type validation, NULL propagation,
   and functional correctness vs known values.
5. **Compatibility debt log.** Curated list of known semantic deltas in
   COMPATIBILITY.md with fixture references and defer rationales for each
   item. Covers force-dimension defaults, empty geometry dimension,
   aggregate-only collect, SRID-less WKB, spheroid WGS84-only, and deferred
   functions.

### Milestone 5 — canonical backend hardening and high-value gaps — ✅ LANDED

Outcome: hard geometry/raster/geography behavior is delegated to the best
available engine, and high-value missing functions are added only when they can be
made faithful.

Landed:

1. **GEOS overlay fallback.** All four scalar overlay operations
   (`ST_Intersection`, `ST_Union`, `ST_Difference`, `ST_SymDifference`) now
   use a `catch_unwind` guard around the local `geo::BooleanOps` fast path,
   falling back to GEOS overlay (the canonical PostGIS engine) when the local
   crate panics on complex or pathological input. The local fast path stays
   the default for common valid geometry; GEOS handles the edge cases.
2. **ST_DumpRings.** Table function `(BLOB) → (path, geom)` returning one row
   per polygon ring. Path `{0}` = exterior ring, `{1}+` = interior rings.
   Handles Polygon, MultiPolygon, and GeometryCollection inputs.
3. **ST_ContainsProperly.** DE-9IM `T**FF*FF*` predicate via `geo::Relate`.
   Returns true when `b` intersects the interior of `a` but not the boundary.
4. **Performance budget extension.** `benchmarks/perf_budget.sql` now includes
   GEOS overlay fallback timing (bowtie polygons), ST_ContainsProperly, and
   ST_DumpRings sections alongside the existing 7 backend families.
5. **SpatialBench snapshot script.** `benchmarks/snapshot.sh` runs the heavy
   workload tier and emits a structured markdown report with metadata
   (commit, DuckDB version, platform), query timings, and catalog audit.
   Designed as a manual/nightly release QA gate.
6. **Milestone 5 fixtures.** `tests/reference/m5_fixtures.sql` with 21 checks:
   ST_ContainsProperly (interior/boundary/exterior/poly/null), ST_DumpRings
   (polygon+hole/simple/multipolygon/non-polygon), GEOS overlay fallback
   (bowtie intersection/union/difference/symdifference), valid overlay
   correctness, NULL propagation.

### Milestone 6 — release-grade usability and maintainability — ✅ LANDED

Outcome: the project is easy for users to install/use and easy for contributors to
extend without breaking fidelity.

Landed:

1. **Contributor contract.** New `CONTRIBUTING.md` with backend decision tree
   (SedonaDB bridge vs GEOS vs local `geo` vs GDAL/PROJ), step-by-step function
   addition walkthrough, test/doc requirements per backend, and semantic delta
   documentation rules.
2. **Registry hygiene.** Dormant function inventory added to `functions.rs`
   listing all local implementations that are superseded by literal SedonaDB
   routing.
3. **User workflow hardening.** `tests/reference/m6_fixtures.sql` with 123
   checks covering 10 workflow families: GeoParquet ingest, CRS transform join,
   bbox prefilter join, dissolve, dump family, raster sampling/reclassification,
   geodesic distance, overlay fallback, PostGIS migration patterns, and literal
   routing parity.
4. **Smoke test hardening.** Expanded from 11 to 15 checks: added overlay
   intersection, DumpRings, ContainsProperly, and literal routing parity.
5. **Versioned release notes.** `CHANGELOG.md` with catalog counts, known deltas,
   deferred items, migration guide (7 patterns), build dependencies, and
   verification summary.

## Completed milestone cycle (Milestones 7–13)

Cycle principles (carried forward from earlier cycles):

- **Compatibility evidence over breadth.** Every new or rerouted `st_*` function
  needs a normal-case fixture, an edge-case fixture, and either literal SedonaDB
  parity or a documented PostGIS/SedonaDB delta.
- **Canonical engines first.** SedonaDB bridge for SedonaDB kernels, GEOS for
  planar topology/validity/overlay, GeographicLib/Karney for spheroid geodesics,
  PROJ for CRS, GDAL for raster I/O, and DuckDB SQL for relational/raster algebra.
- **One obvious public namespace.** Users should reach for familiar `ST_*` names.
  `sedona_st_*` remains the literal/debug/provenance namespace; extension-specific
  helpers need explicit workflow justification.
- **No semantic snowflakes.** Prefer shared registration/executor families,
  generated audits, and small backend boundaries over one-off implementations.
- **Usability is part of fidelity.** Package/load diagnostics, examples,
  migration notes, and smoke tests must stay current with capability changes.

### Milestone 7 — compatibility evidence and namespace closure — ✅ LANDED

Outcome: the project has a cleaner, better-tested compatibility contract and a
shorter list of surprising SQL namespace gaps.

Landed:

1. **Compatibility audit tool.** `tools/catalog_audit.py --compat-check`
   cross-references `sedona_st_*` bridge functions against public `st_*`,
   reporting undocumented routing decisions (local `st_*` with a literal twin
   but no explicit routing/intentionally-local decision). Catches the class of
   doc/code drift where COMPATIBILITY.md claims routing but the registry is
   local.
2. **Literal routing fixes.** `ST_LineSubstring` routed to literal SedonaDB
   kernel (COMPATIBILITY.md already claimed routing — this fixes the drift).
   `ST_GeomFromWKT` added as SedonaDB-naming alias for `ST_GeomFromText`,
   routed to literal kernel. Routed count went 47→49.
3. **High-value namespace gaps.** `ST_SetPoint(linestring, index, point)`
   — replaces point at 0-based index, matching PostGIS. `ST_IsValidDetail`
   — table function `(BLOB) → (valid BOOL, reason VARCHAR, geom BLOB)`,
   matching PostGIS record-returning pattern. New dispatch executor
   `geom_int_geom_to_geom` for `(BLOB, INT, BLOB) → BLOB` shape.
4. **Reference parity corpus.** `tests/reference/m7_fixtures.sql` with
   32 checks: routing parity for all routed functions (envelope, Z ordinate,
   SRID delta, Force2D, NumPoints, AsText/AsBinary, translate/scale/rotate,
   affine, StartPoint/EndPoint, IsEmpty, FlipCoordinates, Reverse, Segmentize,
   Point/MakePoint, Azimuth, typed WKT constructors), functional tests for
   ST_SetPoint (5 edge cases), ST_IsValidDetail (4 cases), ST_LineSubstring
   parity + functional, ST_GeomFromWKT parity + functional.
5. **Compatibility debt pruning.** `ST_IsValidDetail` and `ST_SetPoint` moved
   from deferred to shipped in COMPATIBILITY.md. Known deltas for local WKT
   Z-coordinate parsing documented via fixtures.

Exit gates:

- `--compat-check` reports zero undocumented routing decisions.
- Every newly routed function has parity fixtures against `sedona_st_*`.
- No public namespace change ships without README/COMPATIBILITY coverage.

## Completed milestone cycle (Milestones 8–13): portability and DuckLake scale

This cycle realigned the roadmap around the two product goals: **PostGIS
workload portability** (Milestone 8) and the **DuckLake-native spatial
lakehouse** (Milestones 9–12), converging in workload-scale validation
(Milestone 13).
Design details, verified DuckLake constraints, and the canonical query pattern
live in [ARCHITECTURE.md](./ARCHITECTURE.md).

Verified constraints this plan is built on:

- DuckLake partitioning supports only `identity`, `bucket(N, col)`, and
  `year/month/day/hour` transforms — **no expression partitioning**. Spatial
  partition keys must be materialized columns.
- DuckLake prunes with per-file column min/max stats (zone maps). Plain DOUBLE
  bbox columns prune well **only when files are spatially clustered**, so a
  space-filling-curve sort key is load-bearing, not cosmetic.
- DuckDB C-API extensions cannot register binary operators; `&&`/`<->`/GiST
  remain documented mechanical rewrites, never emulation.
- DuckLake's catalog owns multi-writer commit semantics. The extension's job is
  deterministic, pure partition-key functions — no shared mutable state.

### Milestone 8 — PostGIS workload portability harness — ✅ LANDED

Outcome: we know exactly how hard real PostGIS SQL is to port, with evidence.

Landed:

1. **PostGIS port harness.** `tests/postgis_port/` with 8 case files covering
   constructors, accessors, DE-9IM predicates, overlay, validity, dump family,
   line editing, and operator/PostgreSQL-ism rewrites. 59 test cases total, all
   passing. Docker-based expected-output generator
   (`generate_expected.sh`) for full regression coverage when Docker is
   available (skippable in CI without Docker).
2. **Coverage of high-value regress families.** Every common PostGIS analysis
   pattern has at least one ported case with the original PostGIS SQL as a
   comment, the expected PostGIS result, and the ported DuckDB SQL.
3. **Migration cookbook.** `docs/MIGRATION.md` with tested rewrites for every
   PostgreSQL-ism: `&&` → bbox columns, `<->` KNN → `ORDER BY st_distance` +
   `LIMIT`, casts/typmods → explicit constructors, `CREATE INDEX … USING gist`
   → layout-column guidance, aggregate name mapping, geography type rewrite,
   and a "what does NOT port" table.
4. **Portability ledger in COMPATIBILITY.md.** Per-family status table fed by
   the harness cases: constructors (8), accessors (11), predicates (11), overlay
   (5 → 4 pass + 1 delta), validity (7), dump (5), line editing (8), operator
   rewrites (6).
5. **Delta discovery.** The harness surfaced a real semantic delta:
   `ST_Contains` on boundary points returns `true` (the `geo` crate is more
   permissive than PostGIS DE-9IM). Documented in the test and COMPATIBILITY.md;
   `ST_Covers` is the correct function for boundary-inclusive containment.

### Milestone 9 — spatial partition key primitives — ✅ LANDED

Outcome: geometry columns can produce deterministic, materialized keys that
DuckLake can partition, bucket, sort, and prune on.

Landed:

1. **`ST_BBoxIntersects(a, b)`** — cheap bbox-only predicate for prefilters.
2. **Cell keys.** `ST_QuadKey(geom, zoom)` (envelope-center Bing quadkey,
   deterministic, NULL for NULL/EMPTY), `ST_GeoHash(geom, precision)`
   (PostGIS-compatible base-32 geohash of envelope center). CRS contract: lon/lat
   EPSG:4326 assumed; out-of-range → NULL.
3. **Covering cells.** `ST_CoveringQuadKeys(geom, zoom, max_cells)` table
   function returning `(quadkey, tile_x, tile_y)` rows for all cells covered by
   the geometry's envelope. Fails closed (returns 0 rows) when cell count would
   exceed `max_cells`.
4. **Curve sort keys.** `ST_Hilbert(geom, bits)` and `ST_Morton(geom, bits)` —
   space-filling-curve BIGINT sort keys for spatially clustered writes.
   New `geom_int_to_i64` and `int3_to_geom` dispatch executors.
5. **`ST_TileEnvelope(z, x, y)`** — PostGIS-compatible Web Mercator tile bounds
   polygon (lon/lat coordinates).
6. **Fixtures.** 28 checks covering determinism, locality (nearby points have
   closer Hilbert values), NULL/empty safety, zoom-0 edge case, fail-closed
   covering, and the canonical three-stage layout query pattern.
7. **Architecture.** New `src/spatial_keys.rs` module with pure, deterministic
   core math (tile conversion, quadkey, geohash encoding, Hilbert/Morton curves)
   and inline unit tests.

### Milestone 10 — DuckLake spatial layout recipes — ✅ LANDED

Outcome: a copy-paste path from raw geometry to a partitioned, prunable
DuckLake table.

Landed:

1. **`EXAMPLES_DUCKLAKE.md`.** Seven tested recipes: create with layout columns,
   append, three-stage range query, spatial join, KNN, partition evolution, time
   travel. Plus object-size guidance (100 MB–1 GB Parquet objects), cardinality
   guidance, and common-mistakes list.
2. **End-to-end DuckLake round-trip test.** `tests/reference/m10_ducklake.sql`
   with 9 checks covering table creation, partitioned append, three-stage query
   correctness (cell-pruned = exact-only), partition evolution (zoom → bucketed),
   and time travel (`AT (VERSION => 1)`). Uses in-memory DuckLake catalog for
   CI portability.
3. **Layout benchmark.** `benchmarks/layout_benchmark.sh` comparing three
   layouts (unpartitioned / bbox+sorted / cell-partitioned+sorted) with timing
   output via DuckDB `.timer`. Verified all three return identical row counts.
4. **Critical correctness insight documented.** Covering cells must cover the
   query area bbox (including distance threshold), not just the query point.
   Benchmark initially returned wrong counts when covering only the point —
   fixed and documented as the #1 common mistake.
5. **Ease-of-use macro pack.** `sql/ducklake_spatial_macros.sql` adds optional
   helper macros (`sedona_layout_cell`, `sedona_layout_sort`,
   `sedona_covering_cells_bbox`, `sedona_bbox_overlaps`) so users can keep
   deterministic layout columns but write less boilerplate SQL.

### Milestone 11 — Sedona-style adaptive partitioning — ✅ LANDED

Outcome: skewed datasets get better layouts than a fixed grid.

Landed:

1. **Estimation helpers.** `ST_EstimatePartitionCount(total_rows,
   avg_row_bytes, target_object_bytes)` → INT and `ST_RecommendZoom(
   n_partitions)` → INT. Pure math functions for sizing partition
   strategies from data statistics.
2. **Sort-then-pack adaptive spec recipe.** Tested SQL recipe: compute cell
   histogram at fine zoom → sort cells by quadkey (preserves locality) →
   cumulative sum → cut partitions at target row boundaries → plain
   `(partition_id, cell_min, cell_max, total_rows)` rows. No hidden index
   files, no extension state.
3. **Cell-to-partition lookup.** Direct `JOIN` workflow assigns each
   geometry to its adaptive partition via a small lookup table. No correlated
   subqueries needed.
4. **Skewed-data validation.** 16-check fixture with a 90%-clustered dataset:
   fixed zoom-4 grid produces a hot cell with >400 rows; adaptive spec with
   target=200 produces balanced partitions with smaller max partition size.
5. **Determinism proven.** Same input → same partition spec across repeated
   runs (verified by fixture).

### Milestone 12 — multi-writer DuckLake validation — ✅ LANDED

Outcome: spatial layouts are proven under DuckLake's concurrency model.

Landed:

1. **Three-process multi-writer test.** `tests/reference/m12_multiwriter.sh`
   spawns three separate DuckDB processes, each appending to the same spatial
   DuckLake table with partitioned layout columns. Verified: 1000 rows (500+300+
   200), 1000 distinct IDs (no loss, no duplicates), 6 distinct cells.
2. **Partition evolution round-trip.** Writer A creates at identity partition;
   layout changes to `bucket(4, spatial_cell)`; Writer D appends 50 rows under
   new layout. Total = 1050 rows. Time travel `AT (VERSION => 1)` = 500. Query
   across mixed layouts returns correct results (68 rows near origin).
3. **Concurrency documentation.** `EXAMPLES_DUCKLAKE.md` §8 documents: what
   the extension guarantees (pure functions, no shared state), what DuckLake
   guarantees (catalog commits, conflict resolution), catalog choice matrix
   (DuckDB file = serialized, PostgreSQL = concurrent), and failure modes
   (commit conflict → retry, partition evolution → mixed-layout queries stay
   correct).
4. **CI-compatible.** Test script is skippable via `SEDONA_SKIP_DUCKLAKE=1`;
   uses temp directory with cleanup trap.

### Milestone 13 — workload-scale validation — ✅ LANDED

Outcome: porting and scaling evidence is visible and reproducible.

Landed:

1. **Five ported workload templates.** `tests/reference/m13_workloads.sql`
   with 11 checks across 5 representative PostGIS workloads on DuckLake:
   points-in-polygons spatial join, KNN nearest-neighbor, dissolve/aggregate
   (`ST_Collect`), spatial range query with partition pruning, and bbox window
   query. Each shows PostGIS source SQL, ported DuckDB SQL, and row-count
   parity between exact and pruned variants.
2. **DuckLake partition pruning evidence.** The range-query workload proves
   the three-stage pattern returns the same count as exact-only, and that
   covering cells are a strict subset of all partition cells (observable file
   pruning).
3. **Porting report.** `docs/WORKLOAD_REPORT.md` with per-workload SQL
   comparison, rewrite-effort table, partition-pruning evidence summary, and
   pointers to all test artifacts.

Exit gates met:

- Every workload reports row-count parity between PostGIS-equivalent and
  DuckDB-ported SQL.
- Range-query workload demonstrates DuckLake partition pruning end to end.

## Standing release gates

These are non-negotiable for every milestone and every release:

- No silent wrong geometry.
- `st_*` remains the ergonomic PostGIS/SedonaDB-like namespace.
- Literal SedonaDB kernels remain callable and tested under `sedona_st_*`.
- All backends fail closed on invalid/unsupported input.
- SQL regressions, Rust tests, release packaging, and smoke tests pass.
- New semantic deltas are documented before they ship.

## Target acceptance gates

The extension reaches the north-star target when all of these are true:

1. **SedonaDB surface closure.** Every practical Apache SedonaDB vector SQL
   kernel is either exposed literally, routed from public `st_*`, or listed in
   an intentionally-local / intentionally-deferred ledger with a fixture-backed
   reason.
2. **PostGIS analysis portability.** Common PostGIS vector/geography/CRS/raster
   analysis SQL ports with mechanical rewrites only. Every non-mechanical rewrite
   is documented in `docs/MIGRATION.md` and covered by a test or workload.
3. **No silent semantic deltas.** Known deltas have reference fixtures and
   COMPATIBILITY entries; unknown deltas are treated as release blockers once
   discovered.
4. **DuckLake PB/trillion-row pattern is reproducible.** The project ships a
   scale harness that proves layout creation, append, mixed-layout reads,
   partition pruning, bbox zone-map pruning, and exact-predicate correctness at
   increasing scale factors with 100 MB–1 GB object-size guidance.
5. **Packageable and diagnosable.** Supported extension artifacts load cleanly in
   fresh DuckDB installs; platform gaps are explicit; GEOS/GDAL/PROJ/LLVM
   dependency issues have actionable diagnostics.

## Target completion plan (Milestones 14–20): reached in v2.0.0

This closure slice turned the broad prototype into a target-ready product. The
ordering was deliberate: semantic correctness first, then surface closure, then
scale evidence, then release hardening.

Status after Milestone 20:

- **All 5 target acceptance gates met.** See the gate audit table in
  CHANGELOG.md v2.0.0.
- **Closure slice M14–M20 landed**: geometry fidelity, SedonaDB bridge closure,
  raster/output-format closure, PB-scale DuckLake harness, packaging/CI,
  migration tooling, and release-candidate audit.
- **Foundation slice M7–M13 landed earlier**: compatibility evidence, PostGIS
  portability, spatial partition keys, DuckLake layout recipes, adaptive
  partitioning, multi-writer validation, and workload-scale validation.
- **Remaining work is post-target.** It is useful for adoption and breadth, but
  not required for the v2.0 target unless a new unknown semantic delta is found.

### Milestone 14 — canonical geometry fidelity hardening — ✅ LANDED

Outcome: the highest-risk geometry semantics are aligned with canonical engines
or documented as fixture-backed deltas.

Landed:

1. **`ST_Relate` via GEOS.** Two overloads: `ST_Relate(a, b) → VARCHAR`
   (9-character DE-9IM matrix) and `ST_Relate(a, b, pattern) → BOOLEAN`
   (pattern match with wildcards). GEOS is the canonical PostGIS engine for
   DE-9IM relate. New dispatch executors `binary_geom_varchar` and
   `geom_geom_str_predicate`.
2. **Predicate fidelity audit.** 12 DE-9IM matrices verified against PostGIS
   reference output (interior/boundary/exterior points, equal/overlapping/
   touching/disjoint polygons, line crossing, identical points). Pattern
   matching verified for contains, overlaps, disjoint, and exact matrix cases.
   Adversarial edge cases for all major predicates (Covers, CoveredBy, Touches,
   Crosses, Overlaps, Equals, Disjoint).
3. **`ST_Contains`/`ST_Within` boundary delta PINNED.** PostGIS returns `false`
   when a point lies on the polygon boundary (DE-9IM requires interior
   intersection); our PNPOLY ray-cast is boundary-inclusive on exact vertices.
   Documented in COMPATIBILITY.md debt log with `ST_Covers`/`ST_CoveredBy` as
   the correct substitutes. `ST_ContainsProperly` matches PostGIS semantics.
4. **Adversarial overlay fixtures.** Bowtie (self-intersecting) make_valid +
   intersection workflow, sliver polygons, holes touching shells, collapsed
   rings, polygon-with-hole difference, adjacent-polygon union, symmetric
   difference. 48 total checks in `tests/reference/m14_fixtures.sql`.

Exit gates met:

- No undocumented predicate/overlay delta remains in the audited corpus.
- `tools/catalog_audit.py --compat-check` stays clean.
- SQL fixture count increased; current baseline is tracked in the verification
  summary above.

### Milestone 15 — SedonaDB bridge closure and generated compatibility ledger — ✅ LANDED

Outcome: SedonaDB compatibility is a mechanically-audited contract, not a manual
claim.

Landed:

1. **Bridge inventory refresh.** Every kernel in upstream
   `sedona-functions/src/register.rs` is classified: 44 routed (public `st_*`
   routes to literal kernel), 6 intentionally-local (documented reason), 21
   bridge-only (`sedona_st_*` with no public counterpart), 10 not-bridgeable
   (table fn, aggregate, geography type, or special operator). Zero
   unclassified kernels.
2. **Routing pass closed.** Verified the 6 intentionally-local functions
   (`st_geomfromwkb`, `st_geomfromewkb`, `st_geomfromewkt`, `st_geometryn`,
   `st_pointn`, `st_interiorringn`) cannot be routed: the bridge returns NULL
   for per-row varying integer indices (geometryn/pointn/interiorringn) and
   trust-boundary constructors need local validation. 46 routed, 0 undocumented.
3. **Generated compatibility ledger.** `tools/catalog_audit.py
   --generate-ledger` emits `docs/SEDONA_LEDGER.md` — a stable Markdown table
   classifying every upstream SedonaDB kernel and every live-registry function
   by provenance. Run in CI to detect drift.
4. **CI drift gate.** `ci/check.sh` runs catalog drift check, compat check, and
   ledger freshness check. Fails if docs, registry, or ledger are out of sync.
5. **Parity fixtures.** `tests/reference/m15_fixtures.sql` with 30 checks:
   25 routed-function parity tests (`st_* == sedona_st_*`), 4
   intentionally-local correctness tests (including per-row varying integer
   index), and 1 bridge inventory verification.

Exit gates met:

- SedonaDB bridge inventory has zero unclassified functions.
- Public/local overlap requires an intentionally-local reason.
- README/ROADMAP/COMPATIBILITY counts are drift-checked.
- Ledger freshness is verified in CI.

### Milestone 16 — raster and output-format closure — ✅ LANDED

Outcome: remaining high-value PostGIS analysis facades are either shipped or
explicitly replaced by documented DuckDB-native workflows.

Landed:

1. **`ST_AsSVG(geom)` shipped.** SVG path data string with Y-flipped absolute
   coordinates, matching PostGIS `ST_AsSVG(geom, 0)`. Handles Point,
   MultiPoint, LineString, MultiLineString, Polygon (with holes),
   MultiPolygon (semicolon-separated), and GeometryCollection.
2. **Raster facade decisions.** `ST_Clip`: documented as a DuckDB-native SQL
   workflow — filter `ST_PixelData` by computed geographic bbox using the
   GeoTransform from `ST_RasterTransform`. No raster-returning facade needed.
   `ST_AsRaster`: deferred — needs GDAL rasterization write path.
3. **Output format deferral.** `ST_AsTWKB`: deferred (needs varint delta
   encoding). `ST_AsKML`: deferred (needs CRS transform + KML XML writer).
   `ST_AsMVT`: deferred (needs protobuf encoder + tile clipping). Each has a
   documented workaround fixture (`ST_AsBinary` for TWKB, `ST_AsGeoJSON` for
   KML/MVT) and a clear ship/defer decision in COMPATIBILITY.md.
4. **Raster QA corpus.** `tests/reference/m16_fixtures.sql` now has 25
   checks: SVG output, GeoJSON consistency, clip workflow, dimensions, nodata
   value, origin coordinates, pixel size, stats range, GeoTransform origin,
   pixel count, first-pixel value, in-bounds sampling, out-of-bounds NULL,
   nodata NULL, and deferred-format workaround checks.
5. **Map algebra boundary.** DuckDB SQL remains the expression engine — no
   custom raster language. All raster facades are I/O/streaming helpers.

Exit gates met:

- Every raster/output item in COMPATIBILITY has a ship/defer decision.
- New GDAL paths fail closed on missing files, nodata, and out-of-bounds reads.

### Milestone 17 — PB-scale DuckLake validation harness — ✅ LANDED

Outcome: the spatial lakehouse story is reproducible beyond toy fixtures.

Landed:

1. **Deterministic scale-factor generator.** Point dataset with 80% uniform
   spread + 20% clustered near origin, at smoke (1k), local (10k), and heavy
   (100k) tiers. Same SQL and data model at every tier.
2. **Three-layout comparison harness.** `benchmarks/scale_harness.sh` creates
   flat, bbox+Hilbert-sorted, and cell-partitioned+Hilbert-sorted DuckLake
   tables. Reports file counts, distinct cells, candidate rows, cell-pruning
   ratio, and exact-result parity. Verified at smoke and local tiers.
3. **Scale-tier SQL fixtures.** `tests/reference/m17_scale.sql` with 11
   checks: row-count parity, range-query parity, cell/bbox pruning
   effectiveness, join parity, KNN parity, cell cardinality, adaptive
   partitioning balance, partition evolution correctness, append correctness,
   and time-travel correctness.
4. **Scale evidence report.** `docs/SCALE_REPORT.md` with tier comparison,
   pruning ratio analysis (35%→22% as data grows), extrapolation guidance,
   and known limitations.

Exit gates met:

- Local tier proves cell pruning + bbox zone-map pruning + exact correctness
  end to end on DuckLake.
- Reports are deterministic and include layout metadata for extrapolation.
- Exact predicate result is always the oracle; no benchmark shortcut affects
  correctness.

### Milestone 18 — packaging, CI, and load diagnostics — ✅ LANDED

Outcome: users can install, load, and debug the extension without reproducing the
development environment.

Landed:

1. **Unified CI pipeline.** `ci/all-checks.sh` runs all five quality gates in
   sequence: Rust unit tests, drift gate (catalog/compat/ledger), SQL regression
   suite (including DuckLake), package-and-smoke, and scale harness smoke tier.
   One command, exits non-zero on any failure.
2. **Smoke-test hardening.** Expanded from 15 to 18 backend checks: added
   `ST_Relate` matrix output (GEOS), `ST_Relate` pattern matching, and
   `ST_AsSVG` output format. Every backend family (local, SedonaDB bridge,
   aggregates, GEOS, spheroid, raster, PROJ, table functions, overlay,
   DumpRings, ContainsProperly, routing parity, relate, SVG) is validated from
   a packaged `.duckdb_extension`.
3. **SQL runner strictness.** `tests/run_sql.sh` now detects DuckDB parser,
   binder, catalog, and runtime errors as test failures — not just explicit
   `FAIL` rows. Prevents silent SQL errors from passing as green.
4. **Dependency diagnostics.** `docs/DEPENDENCIES.md` with supported version
   matrix (GEOS ≥ 3.8.0, GDAL ≥ 3.5, PROJ ≥ 9.0, LLVM ≥ 14), build environment
   setup (Linuxbrew + container), runtime load error troubleshooting table, and
   packaging instructions for all supported platforms.

Exit gates met:

- Smoke test validates every backend from a packaged extension in one command.
- Failure messages point to the missing dependency and expected remedy.
- CI catches stale docs/ledger, SQL errors, packaging failures, and missing
  runtime libraries.

### Milestone 19 — migration UX and operator-rewrite tooling — ✅ LANDED

Outcome: PostGIS users get a boring, repeatable migration path.

Landed:

1. **Conservative SQL rewrite linter.** `tools/postgis_rewriter.py` scans
   `.sql` files for PostGIS-specific patterns (`&&`, `<->`, `<#>`, `::geometry`,
   `::geography`, `geometry(Type,SRID)` typmods, `USING gist`,
   `ST_Union(geom)` aggregate, `ST_MemUnion`, `ST_AsMVT/TWKB/KML`,
   scalar `ST_Collect`, `ST_DWithin` on geography) and produces annotated
   output with line-level suggestions. High-confidence patterns have mechanical
   rewrites; low-confidence patterns are flagged for human review. Never changes
   query semantics silently.
2. **Machine-readable JSON export.** `tools/catalog_audit.py --export-json`
   emits `docs/sedonadb_compat.json` — counts, classifications, routed
   functions, bridge-only functions, intentionally-local reasons, and the full
   upstream SedonaDB inventory. Shares the same source as the Markdown ledger
   so docs, tools, and release notes always agree.
3. **Migration workbook.** `docs/MIGRATION.md` expanded with 5 end-to-end
   examples: complete DDL + query migration, DuckLake layout migration (GiST →
   layout columns + partitioning), invalid geometry handling, raster sampling
   workflow, and automated rewriter usage guide.
4. **Macro-dependent test phase.** `tests/run_sql.sh` now runs a third phase
   for tests that require the optional DuckLake spatial macros. 13-check
   fixture (`tests/reference/m19_fixtures.sql`) verifies rewritten PostGIS
   patterns and macro behavior.

Exit gates met:

- Rewriter never changes query semantics silently; uncertain patterns are
  warnings.
- Migration cookbook examples run as tests.
- JSON compatibility export and Markdown ledger agree with the live registry.

### Milestone 20 — target release candidate — ✅ LANDED

Outcome: the project is ready to call the north-star target reached for the
supported scope.

Landed:

1. **Target audit.** All 5 target acceptance gates reviewed and confirmed met:
   - SedonaDB surface closure (M15: 81 kernels classified, 0 unclassified)
   - PostGIS analysis portability (M8/M13/M19: port cases, workloads, rewriter)
   - No silent semantic deltas (M14: all deltas fixture-backed)
   - DuckLake PB/trillion-row pattern (M17: scale harness with parity evidence)
   - Packageable and diagnosable (M18: unified CI, dependency docs)
2. **Release notes.** CHANGELOG.md v2.0.0 with full milestone-by-milestone
   summary, gate audit table, known deltas, the two active backlog items with
   workarounds, and verification commands.
3. **Compatibility freeze.** No undocumented semantic deltas, no unclassified
   SedonaDB kernels, no stale catalog counts, ledger and JSON export verified
   fresh.
4. **Clean-checkout release rehearsal.** `ci/all-checks.sh` passes all 5 phases
   from the current checkout.

Exit gates met:

- All target acceptance gates pass.
- Standing release gates pass from a clean checkout.
- Remaining active backlog items are explicitly non-blocking with documented
  workarounds.

## Completed post-target plan: v2.1 production-readiness and delta reduction

The v2.0 target is reached for the supported scope, and the README target has
user demand. This target made the extension boring to consume outside this dev
checkout, kept generated evidence fresh, and retired high-impact non-blocking
gaps where canonical implementations were available.

Acceptance gates for this target:

1. **Distribution matrix proven.** Linux and macOS artifacts build, package, load,
   and pass smoke with documented dependency diagnostics.
2. **Known deltas stay intentional.** Every existing semantic delta is either
   retired or re-pinned with a fixture and migration guidance; no new undocumented
   delta ships.
3. **Backlog stays small and target-driven.** No SQL-surface output backlog
   items remain active — all PostGIS output encoders are shipped (SVG, KML,
   TWKB, MVT). `ST_AsRaster` is the only remaining facade gap.
4. **Scale evidence remains reproducible.** Smoke/local/heavy scale tiers keep
   exact-result parity as the oracle; timing remains informational.
5. **Generated docs stay drift-free.** Catalog counts, ledger, JSON export,
   compatibility tables, and release notes agree with the live registry.

### Milestone 21 — release distribution matrix — ✅ LANDED

Outcome: the extension builds, packages, loads, and passes smoke on supported
platforms with automated CI.

Landed:

1. **GitHub Actions CI.** `.github/workflows/ci.yml` runs on every push/PR:
   - `linux-amd64` (ubuntu-22.04): full 5-phase pipeline via `ci/all-checks.sh`
   - `macos-arm64` (macos-14): build + Rust tests + smoke test
   - `lint` (ubuntu-22.04): `cargo fmt --check` + `cargo clippy` (non-blocking)
2. **Release checklist.** `docs/RELEASE_CHECKLIST.md` with step-by-step
   packaging, smoke, checksum, tag, and release instructions.
3. **Platform matrix.** Documented in DEPENDENCIES.md:
   linux_amd64 (CI-tested), darwin_arm64 (CI-tested), darwin_amd64 (buildable).
4. **Dependency setup per platform.** Ubuntu: apt packages; macOS: Homebrew.
   DuckDB CLI downloaded from GitHub releases.

Exit gates met:

- Each supported platform artifact loads in a fresh DuckDB session.
- Dependency failures point to actionable fixes.
- Unsupported platforms (Windows) are explicitly labeled.

### Milestone 22 — semantic-delta retirement pass — ✅ LANDED

Outcome: the highest-visibility PostGIS semantic delta is retired; the
compatibility debt log shrinks from 6 to 5 entries.

Landed:

1. **`ST_Contains`/`ST_Within` boundary delta retired.** Both predicates now
   route through `geo::Relate` with the PostGIS DE-9IM pattern `T*****FF*`
   instead of the old PNPOLY ray-cast. Boundary points now return `FALSE`
   (matching PostGIS DE-9IM), interior points still return `TRUE`. `ST_Covers`
   and `ST_CoveredBy` remain the correct boundary-inclusive substitutes.
2. **Fixture updates.** `tests/postgis_port/cases/03_predicates.sql` boundary
   case now expects `FALSE` (was a pinned delta expecting `TRUE`).
   `tests/reference/m14_fixtures.sql` §4 rewritten from "PINNED DELTA" to
   "RETIRED DELTA" with before/after verification.
3. **New M22 fixture file.** `tests/reference/m22_fixtures.sql` with 14
   checks: interior, boundary vertex, boundary edge midpoint, exterior, polygon
   contains polygon, shared-edge edge case, covers boundary, ContainsProperly,
   NULL propagation, empty operand.

Exit gates met:

- No changed semantic behavior ships without fixture proof and docs.
- Compatibility debt log shrinks; no stale or ambiguous entries for
  `ST_Contains`/`ST_Within`.

### Milestone 23 — raster and output-format breadth — ✅ LANDED

Outcome: two active output-encoder backlog items (`ST_AsKML`, `ST_AsTWKB`)
shipped, leaving only `ST_AsMVT` (protobuf + tile clipping) in the active
backlog.

Landed:

1. **`ST_AsKML(geom)` shipped.** KML 2.2 XML serialization for Point,
   MultiPoint, LineString, MultiLineString, Polygon (with holes),
   MultiPolygon, and GeometryCollection. Coordinates as `lon,lat`.
2. **`ST_AsTWKB(geom)` shipped.** Tiny WKB compact binary encoding with
   zigzag varint delta encoding, precision=0, no bbox/size. Returned as
   lowercase hex string (use `unhex()` for binary BLOB). Handles all
   geometry types.
3. **Active output backlog shrinks: 3 → 1.** Only `ST_AsMVT` remains —
   it genuinely needs a protobuf encoder and tile-clipping logic.
4. **13-check fixture** covering KML and TWKB for all geometry types.

Exit gates met:

- Every shipped format has deterministic fixtures and documented CRS/unit rules.
- Remaining backlog format (`ST_AsMVT`) keeps tested workarounds in COMPATIBILITY
  and MIGRATION docs.

### Milestone 23b — ST_AsMVT output encoder — ✅ LANDED

Outcome: the last output-encoder backlog item is shipped. All PostGIS output
formats are now supported: Text, Binary, EWKB, GeoJSON, HexEWKB, SVG, KML,
TWKB, and MVT.

Landed:

1. **`ST_AsMVT(geom)` shipped.** Mapbox Vector Tile encoding as hex-encoded
   protobuf. Creates a single-layer, single-feature MVT tile using the MVT 2.1
   spec geometry command encoding (MoveTo/LineTo/ClosePath with zigzag-varint
   deltas). Handles Point, MultiPoint, LineString, MultiLineString, Polygon
   (with holes), and MultiPolygon. Coordinates must be in tile-local integer
   space (0..4096).
2. **Protobuf encoding** implemented from scratch — no external protobuf
   dependency. Field tags, varints, length-delimited messages, packed repeated
   uint32 all hand-rolled.
3. **10-check fixture** covering all geometry types, NULL propagation,
   protobuf tag verification, and layer-name verification.
4. **Active output backlog: 1 → 0.** Only `ST_AsRaster` (GDAL write path)
   remains as a non-output facade gap.

### Milestone 24 — scale operations and workload guardrails — ✅ LANDED

Outcome: DuckLake scale evidence is reproducible, and the PostgreSQL-catalog
multi-writer path is validated with a local container.

Landed:

1. **Scale report generator.** `benchmarks/scale_report.sh` runs the scale
   harness at smoke + local tiers, collects metrics (row counts, candidate
   counts, pruning ratios, parity), and emits a structured Markdown report
   suitable for release QA. Exact-result parity is the only correctness oracle.
2. **PostgreSQL-catalog multi-writer validation.**
   `tests/reference/m24_pg_catalog.sh` starts a PostgreSQL 16 container,
   attaches DuckLake with a PostgreSQL catalog, and validates:
   - Three-process sequential append (1000 rows, no duplicates)
   - Three-stage query parity (cell + bbox + exact = exact-only)
   - Time travel (`AT (VERSION => 1)`)
   - Partition evolution (quadkey → bucketed)
   - Discovered and documented: PostgreSQL reserves `xmin`/`xmax` as system
     column names — use `minx/miny/maxx/maxy` for bbox columns with PG catalogs.
3. **Object-store deployment guidance.** `docs/SCALE_REPORT.md` expanded with
   file-sizing, partition-cardinality, Hilbert-clustering, adaptive-spec, and
   catalog-choice recommendations for PB-scale deployments.
4. **Monitoring guidance.** SQL examples for checking zone-map effectiveness
   via `ducklake_file_column_stats`.

Exit gates met:

- Scale reports include row counts, candidate counts, pruning ratios, and parity.
- Workload recipes remain copy-pasteable and explain when pruning affects only
  speed, never correctness.
- PostgreSQL-catalog multi-writer path is validated end-to-end.

## Next target: v2.2 Rust-first migration and upstream coverage

The next target is to make PostGIS + SedonaDB migration and verification mostly
mechanical while reducing local/custom test burden. The core rule: prefer
upstream PostGIS/SedonaDB fixtures and a shared Rust AST rewrite engine over
hand-written one-off migration scripts and bespoke local fixtures.

Reference-repo findings that shaped this plan:

- **PostGIS** (`postgis/postgis`): `regress/core/*.sql` contains broad, stable
  SQL fixture coverage for predicates (`relate.sql`, `regress_ogc*.sql`),
  overlay (`union.sql`, `difference.sql`, `symdifference.sql`), processing
  (`simplify.sql`, `snap.sql`, `subdivide.sql`), output (`twkb.sql`), and CRS
  (`regress_proj_*.sql`). These are the best source for compatibility edge
  cases, but require PostgreSQL syntax rewriting and expected-output adaptation.
- **DuckDB Spatial** (`duckdb/duckdb-spatial`): test suites under
  `test/sql/postgis/`, `test/sql/geos/`, `test/sql/geometry/`, and
  `test/sql/mvt/` show a useful precedent: organize by backend/function family
  and test MVT through round-trip metadata reads. Its `ST_AsMVT` is richer than
  our scalar encoder (record/aggregate-style layer properties), so it is a good
  follow-up reference for multi-feature MVT UX.
- **DuckLake** (`duckdb/ducklake`): tests under `test/sql/transaction/`,
  `time_travel/`, `data_inlining/`, and `partitioning/` validate conflict and
  metadata semantics. Our spatial DuckLake tests should import their transaction
  patterns rather than invent new catalog semantics.
- **sqlparser-rs** (`apache/datafusion-sqlparser-rs`): PostgreSQL dialect tests
  parse `&&` as `BinaryOperator::PGOverlap`, support custom operators, and parse
  PostgreSQL casts. This validates the Rust AST rewriter plan; it can correctly
  transform complex operands where regex corrupts expressions.
- **sqlglot** (`tobymao/sqlglot`): excellent Python reference for transpiler UX
  (`parse_one`, `transpile`, unsupported-level controls), but not suitable as the
  extension runtime dependency. Use it as design inspiration only.
- **SedonaDB** (`apache/sedona-db`): Rust-side tests are concentrated around
  WKB/geo-traits and test harness helpers rather than a large SQL fixture corpus;
  bridge parity should therefore continue to use generated kernel inventory plus
  curated SedonaDB fixtures where they map cleanly.

Acceptance gates for the next target:

1. **Shared Rust rewrite engine.** A `sqlparser-rs`-based PostGIS→DuckDB
   rewriter exists in Rust and is shared by CLI tooling, SQL functions, and
   upstream test import. Regex remains only a fallback/quick linter.
2. **Usable migration surfaces.** Users get a CLI (`sedonadb-rewrite`) for file
   migration, plus SQL functions/table functions for notebook/workbook usage and
   review reports. No claim of transparent interception: DuckDB must parse SQL
   before extension code runs.
3. **Upstream-first testing.** PostGIS/SedonaDB fixtures are imported/ported
   where feasible; local custom SQL fixtures remain only for DuckDB-specific
   behavior, discovered deltas, and regression reproductions.
4. **Risk-tiered test execution.** Low-risk deterministic fixtures are
   consolidated and/or parallelized; stateful/high-risk tests (DuckLake,
   PostgreSQL catalog, packaging, scale harness) stay isolated.
5. **No masked crashes.** The aggregate `ORDER BY` segfault discovered during
   batching is fixed or test-isolated with an explicit failing fixture; runner
   failures include process exit status, not just PASS/FAIL row counts.

### Milestone 25 — Rust AST PostGIS rewriter — ✅ LANDED

Outcome: one canonical rewrite engine written in Rust, using `sqlparser-rs`
(already in the dependency tree via DataFusion), replaces regex as the source
of truth for mechanical PostGIS migration.

Landed:

1. **`src/rewriter.rs`** with AST transforms via `VisitorMut`:
   - `a && b` → `st_intersects(a, b)` (PostgreSQL `PGOverlap` binary op)
   - `a <-> b` / `a <#> b` → `st_distance(a, b)` (custom binary op, when tokenized)
   - `::geometry` / `::geography` casts → unwrapped + warning for geography
   - `ST_MemUnion` → `ST_Union_Agg`
   - `CREATE INDEX ... USING gist` → warning (no DuckDB equivalent)
2. **`sedonadb_rewrite_postgis(sql)` SQL function** registered in DuckDB.
   Returns rewritten SQL with confidence annotations.
3. **8 Rust unit tests** + **8 SQL fixture checks** validating all transform
   rules, complex expressions, NULL propagation, and clean-SQL passthrough.
4. **Parse-error handling**: unparseable SQL is returned unchanged with a
   diagnostic comment. The rewriter fails closed.

Exit gates met:

- Complex operands (`a.geom && b.geom`, nested expressions) rewrite correctly
  — no regex-style operand corruption.
- Regex tool (`tools/postgis_rewriter.py`) remains as a quick linter; the Rust
  AST engine is the canonical source of truth.

### Milestone 26 — upstream PostGIS and SedonaDB fixture ingestion — ✅ LANDED

Outcome: upstream PostGIS tests provide direct compatibility evidence for
ST_Relate and ST_Boundary.

Landed:

1. **PostGIS regress relate tests.** `tests/upstream_curated/postgis_relate.sql`:
   40 DE-9IM relate test cases ported from PostGIS
   `regress/core/relate.sql` with expected output from `relate_expected`.
   All 40 matrices match our GEOS-backed ST_Relate exactly — direct
   PostGIS DE-9IM compatibility proof.
2. **PostGIS regress boundary tests.** `tests/upstream_curated/postgis_boundary.sql`:
   6 boundary test cases ported from `regress/core/boundary.sql`.
   Known deltas documented: empty boundary type representation
   (`GEOMETRYCOLLECTION EMPTY` vs `POINT EMPTY`), polygon boundary
   (`MULTILINESTRING` vs `LINESTRING`), and MULTIPOLLECTION/GEOMETRYCOLLECTION
   boundary gaps.
3. **Upstream-first fixture policy** documented in ARCHITECTURE.md: upstream
   sources preferred over bespoke local fixtures; local fixtures only for
   DuckDB-specific shapes and discovered deltas.

Exit gates met:

- At least one curated PostGIS upstream fixture file runs green per major L2
  category (relate: 40/40, boundary: 6/6 with documented deltas).
- Local bespoke fixtures are retained but the upstream tests provide
  independent compatibility validation.

### Milestone 27 — test consolidation and runner hardening — ✅ LANDED

Outcome: local testing catches crashes instead of masking them; test files are
grouped by risk tier; the aggregate ORDER BY segfault is isolated and
documented.

Landed:

1. **Runner crash detection.** `tests/run_sql.sh` now captures DuckDB exit
   codes and reports `CRASH=SIGSEGV` or `CRASH=SIGABRT` as failures. Previously,
   `|| true` silently swallowed process crashes, masking the aggregate ORDER BY
   segfault as a silent "pass".
2. **Risk-tier grouping.** Tests are split into:
   - Tier A: stateless deterministic SQL fixtures
   - Tier B: macro-dependent (requires DuckLake spatial macros)
   - Tier C: DuckLake stateful (requires catalog cleanup)
   Each tier only runs if the previous tier passes.
3. **Aggregate ORDER BY segfault isolated.** The `st_makeline_agg(g ORDER BY k)`
   line in `tests/all_functions.sql` was silently crashing DuckDB, hiding ~15
   subsequent test lines. Fixed by: removing ORDER BY from the test line,
   documenting the bug in `tests/reference/m27_known_issues.sql`, and adding
   runner crash detection so future segfaults are caught.
4. **Known-issues fixture.** `tests/reference/m27_known_issues.sql` documents
   the aggregate ORDER BY segfault, root cause hypothesis, and SQL workaround.

Exit gates met:

- A simulated crash (non-zero DuckDB exit / signal) is reported as a test
  failure, not silently swallowed.
- Standard SQL phase no longer hides the ~15 tests after the crash point.
- `SEDONA_TEST_MODE=isolated` remains available via the per-file loop.

### Milestone 28 — migration assistant UX — ✅ LANDED

Outcome: the Rust rewriter is usable as a real migration assistant, not just a
transpiler. One command produces runnable DuckDB SQL plus an actionable review
report.

Landed:

1. **Structured diagnostics.** `rewrite_postgis_detailed()` returns a
   `RewriteResult` with typed `RewriteEvent`s: kind, confidence (High vs
   NeedsReview), 1-based source line number (via `sqlparser::Spanned`), and a
   human-readable description. The existing `rewrite_postgis()` string API is
   retained as a thin wrapper for the SQL function.
2. **`sedonadb-migrate` CLI** (`src/bin/migrate.rs`):
   - `sedonadb-migrate input.sql --out output.sql --report report.md`
   - Rewritten SQL to `--out` (or stdout if omitted)
   - Markdown report with: per-line rewrites table, confidence levels, items
     requiring manual review, and DuckLake layout hints (bbox columns, spatial
     partitioning, GiST alternatives)
   - Uses `#[path]` to include the rewriter module directly — the binary
     depends only on `sqlparser`, not the extension's GDAL/GEOS/PROJ stack
3. **CI round-trip fixture** (`tests/reference/m28_fixtures.sql`, 13 checks):
   rewrites PostGIS SQL via the SQL function, verifies the rewritten text, then
   executes equivalent DuckDB SQL to prove semantics are preserved.
4. **Migration CLI smoke phase** in `ci/all-checks.sh` (phase 5): builds the
   binary and verifies it produces correct output on a sample.

Confidence model:
- **High**: `&&` → `st_intersects`, `<->` → `st_distance`, `::geometry` cast
  removal, `ST_MemUnion` → `ST_Union_Agg`, `ST_Collect(a,b)` →
  `st_collect_scalar` — exact semantic equivalences.
- **NeedsReview**: `::geography` cast (geodesic distance semantics change),
  `CREATE INDEX ... USING gist` (no DuckDB equivalent).

Exit gates met:

- Users can run one command and get runnable SQL plus an actionable review
  report.
- Low-confidence rewrites are never applied silently — they are flagged with
  warnings in the SQL output and listed in the report.

### Milestone 29 — aggregate ORDER BY crash fix — ✅ LANDED

Outcome: the aggregate ORDER BY segfault (SIGABRT from heap corruption,
documented in M27) is fixed. Users no longer lose their session when a PostGIS
query with `ST_Collect(g ORDER BY k)` reaches DuckDB's C-API aggregate path.

Root cause: DuckDB's C-API aggregate execution does not properly support ORDER
BY inside aggregate function calls. When `AGG(g ORDER BY k)` is parsed, DuckDB
sorts the input but leaves some per-row state slots uninitialized (the `inner`
pointer in `FfiState<T>` is null). The original update callbacks dereferenced
these null inner pointers, corrupting the heap.

Fix (two layers):

1. **Crash-safe update callbacks.** All aggregate update callbacks (`collect`,
   `envelope`, `union`, `makeline`) now check `FfiState::with_state_mut` for
   `None` (null inner) before accessing the state. Uninitialized slots are
   skipped rather than crashing. This prevents the SIGABRT.
2. **Rewriter detection.** `sedonadb_rewrite_postgis()` and the
   `sedonadb-migrate` CLI detect `ORDER BY` inside any aggregate function call
   via `FunctionArgumentClause::OrderBy` and emit a NeedsReview warning with
   the subquery workaround: `AGG(g) FROM (SELECT g FROM t ORDER BY k) sorted`.

The old `tests/reference/m27_known_issues.sql` is replaced by
`tests/reference/m29_fixtures.sql` which proves: the crash is gone, the
rewriter detects the pattern, and the subquery workaround works.

Exit gates met:

- `st_collect(g ORDER BY k)` no longer crashes DuckDB (exit 0, not 134).
- The rewriter detects ORDER BY in aggregates and warns with an actionable fix.
- Aggregate functions without ORDER BY are unaffected (baseline fixtures pass).

### Milestone 30 — upstream PostGIS fixture expansion — ✅ LANDED

Outcome: eight PostGIS regress suites ported as curated upstream fixtures
across two commits: simplify (8), empty (18), measures (15), topology (16),
editing (19), accessors (21). Plus a crash fix for ST_Buffer on EMPTY.

Landed (first batch — simplify, empty, measures):

1. **postgis_simplify.sql** (8 tests): ST_Simplify Douglas-Peucker.
2. **postgis_empty.sql** (18 tests): EMPTY geometry handling.
3. **postgis_measures.sql** (15 tests): ST_Area/Perimeter/Length/Distance.

Landed (second batch — topology, editing, accessors):

4. **postgis_topology.sql** (16 tests): ST_Node (crossing/overlap/self-
   intersect + SRID), ST_Polygonize (basic + multi-ring), ST_Snap (vertex +
   polygon + SRID), ST_VoronoiPolygons (basic + SRID + NULL),
   ST_DelaunayTriangles (basic + 4-point + polygon flag + SRID).
5. **postgis_editing.sql** (19 tests): ST_Reverse (line/multiline/polygon/
   multipoly/point + SRID), ST_Normalize (collection + polygon),
   ST_RemovePoint (middle/first/last + single-segment permissiveness),
   ST_SetPoint (middle/first/last), ST_FlipCoordinates (point/line/polygon),
   ST_ForcePolygonCW (basic + SRID).
6. **postgis_accessors.sql** (21 tests): ST_IsCollection (15 cases —
   singletons false, multi/collection true, empties), ST_HausdorffDistance
   (identical=0, different, polygon, line-multipoint),
   ST_MinimumClearance (polygon + point), ST_IsValid (square, bowtie,
   triangle).

Bug fix: `st_buffer(EMPTY, radius)` panicked in `geo` buffer algorithm.
Now checks `is_empty()` and returns NULL.

Total upstream-curated PostGIS fixtures: 2 → 8 files (relate, boundary,
simplify, empty, measures, topology, editing, accessors).
Total SQL checks: 863 → 962 (+99).

## Idea parking lot

The README target is demand-backed. This parking lot is only for ideas outside
the current target-critical path, or ideas without a credible canonical
implementation path yet:

- S2/H3 cell keys (extension-native, clearly labeled; geohash/quadkey remain the
  default shipped keys).

## Not currently a goal

- PostgreSQL planner/operator compatibility (`&&`, `<->`, GiST hooks) beyond
  documented functional equivalents.
- PostGIS topology schema, Tiger geocoder, address standardizer.
- Full SFCGAL/CGAL 3D solids/surfaces before mature Rust bindings exist.
- A custom raster map-algebra expression language; DuckDB SQL is the expression
  language.

## Definition of done for a new capability

1. Namespace matches PostGIS/SedonaDB where feasible, or a DuckDB-specific name is
   justified.
2. Canonical backend chosen and documented.
3. Invalid/unsupported inputs return NULL or a documented error state; no panic.
4. SQL regression covers normal behavior and at least one edge case.
5. If overlapping SedonaDB exists, fidelity comparison is added or divergence is
   documented.
6. README/ROADMAP mention any semantic delta or runtime dependency.
7. `cargo test --lib`, release build, SQL suites, and package smoke pass.
