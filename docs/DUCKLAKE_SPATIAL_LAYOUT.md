# DuckLake spatial-temporal layout

## Goal

QuackGIS should make large geospatial writes fast without asking users to design
partitions or maintain a secondary index. The target workloads are:

- many parallel writers;
- append-heavy tables that grow to billions or trillions of features;
- large captures such as 10 TB aerial survey batches;
- dense local-coordinate datasets such as CAD/BIM models;
- common map/client queries that filter by area and often by time.

Correctness never depends on the layout. Every spatial predicate is still
rechecked by SedonaDB. The layout only decides which DuckLake partitions, files,
and Parquet row groups can be skipped.

## Decision

Use a **WKB-first, Sedona-bbox layout** for M5. Keep GeoArrow as metadata and a
future native-array optimization, not the primary storage/execution pivot yet.

Why:

- QuackGIS already stores PostGIS-compatible WKB/EWKB in DuckLake/Parquet and on
  pgwire.
- SedonaDB already exposes fast WKB bounds parsing (`wkb_bounds_xy`), which is
  enough to compute layout columns in one geometry pass at write time.
- A local GeoArrow 0.8 probe confirmed that Arrow `Binary` WKB with
  `ARROW:extension:name = geoarrow.wkb` is recognized and row-readable as
  GeoArrow WKB, but it is still serialized WKB: row access parses WKB and
  GeoArrow 0.8 does not provide a better batch bbox primitive for our path.
- Native GeoArrow coordinate arrays may be a later in-memory optimization, but
  they would complicate DuckLake/PostGIS-wire interoperability today.

Clear direction: **persist WKB + hidden layout columns; optionally tag Arrow
fields as `geoarrow.wkb` for interoperability; do not wait for native GeoArrow
geometry arrays before implementing pruning.**

## Spatial type model

QuackGIS should support spatial data in tiers so common PostGIS clients work now
and high-fidelity CAD/reality-capture data can load without destructive
conversion.

### Tier 1: queryable geometry/geography

These are first-class SQL columns, advertised through PostGIS-compatible catalog
metadata and eligible for automatic layout/pruning:

| Type family | Initial storage | Query direction |
|---|---|---|
| `geometry(Point|LineString|Polygon|Multi*|GeometryCollection, SRID)` | WKB/EWKB Binary with `GEOMETRY` catalog type | Full 2D OGC Simple Features path through SedonaDB |
| `geometry(...Z)`, `geometry(...M)`, `geometry(...ZM)` | EWKB keeps Z/M/SRID when supplied | Preserve dimensions; 2D bbox layout uses XY; Z/M predicates are explicit future work |
| `geography(...)` | EWKB plus geodetic CRS metadata, normally EPSG:4326 | Use lon/lat layout for pruning; exact geodesic functions as Sedona/PostGIS compatibility grows |
| Nullable/empty geometry | Null or empty WKB | No spatial bucket pruning; exact predicate decides |

The first implementation should be strict about the queryable contract: if
SedonaDB cannot evaluate a type exactly, QuackGIS can store it but must either
derive a safe 2D query geometry or fail closed for unsupported exact predicates.

### Tier 2: high-fidelity source geometry

CAD/BIM/reality-capture pipelines often contain curves, splines, arcs, meshes,
solids, annotation, blocks, and local engineering coordinates. Flattening these
to simple polygons loses information. The design therefore keeps a dual
representation when importing from high-fidelity sources:

- **`geom`**: queryable 2D/3D WKB geometry derived for spatial indexing, display,
  and ordinary SQL predicates;
- **source sidecar columns**: original format/layer/object id, source CRS, source
  coordinate epoch, vertical datum, transform pipeline, conversion tolerance, and
  optionally the original geometry/object bytes or URI;
- **curve/mesh policy metadata**: whether arcs/splines were preserved, tessellated
  with a tolerance, or represented only by a footprint/envelope.

This makes generated architectural/CAD content loadable immediately while
preserving enough provenance to reprocess it later with better kernels.

### Tier 3: large non-feature assets

Reality-capture projects should not always explode into one SQL row per point or
triangle. QuackGIS should index large assets by metadata and footprint first:

| Asset class | Industry interchange | QuackGIS direction |
|---|---|---|
| Point clouds | LAS/LAZ, COPC, E57, Entwine/EPT | Store asset URI + footprint/3D bounds + time/CRS metadata; optional sampled or tiled point feature tables |
| Orthomosaics/DSM/DTM | Cloud Optimized GeoTIFF, GeoTIFF | Store raster asset URI + footprint + resolution bands; vector layout indexes footprints |
| Meshes/reality models | 3D Tiles, glTF/glb, OBJ/PLY | Store asset URI + tileset/mesh metadata + footprint/3D bounds; derived polygons for query |
| CAD/BIM exchange | IFC, CityGML, LandXML, DXF/DWG via external converters | Store derived feature layers plus source-object provenance and tolerance metadata |

The SQL/vector path remains simple features; the asset path preserves high-volume
data in the formats industry already exchanges and streams.

## Coordinate fidelity over time

High-accuracy aerial/CAD workflows fail when coordinates are treated as timeless
WGS84 numbers. QuackGIS table and column metadata should preserve:

- CRS authority/code and full WKT2/projjson when available;
- horizontal, vertical, and compound CRS components;
- coordinate epoch / acquisition epoch for dynamic datums and tectonic drift;
- source coordinate system name for local engineering grids;
- transform pipeline identifier, grid-shift files, and software/version used;
- stated horizontal/vertical accuracy, units, and conversion/tessellation
  tolerance;
- original source CRS and original coordinates or source-object URI when a
  transform was applied.

Automatic layout uses the normalized query geometry for pruning, but this
metadata makes transforms reproducible and lets future reprocessing correct for
datum updates or coordinate drift without losing the original survey intent.

## Physical layout

Use hidden, deterministic layout columns written with each geometry row. Compute
the geometry-derived columns in one vectorized pass over the WKB batch, not by
calling one SQL UDF per output column.

| Column | Source | Purpose | Physical role |
|---|---|---|---|
| `_qg_minx/_qg_miny/_qg_maxx/_qg_maxy` | Sedona `wkb_bounds_xy` over the geometry WKB | exact bbox facts for the row | ordinary Parquet columns with min/max stats |
| `_qg_space_bucket` | adaptive spatial cell | bounded area partition pruning | DuckLake partition column |
| `_qg_space_sort` | Hilbert key inside the bucket | row-group locality and stable compaction order | ordinary Parquet column, sort key |
| `_qg_time_start/_qg_time_end` | detected/configured time column or interval | temporal overlap pruning | ordinary Parquet columns with min/max stats |
| `_qg_time_bucket` | adaptive time bucket | bounded temporal partition pruning | DuckLake partition column |

Default physical order: `(_qg_time_bucket, _qg_space_bucket, _qg_space_sort)`.
The DuckLake partition spec includes only the two coarse bucket columns. Bbox,
time bounds, and sort keys stay as ordinary data columns so Parquet row-group and
file statistics can do fine pruning without exploding DuckLake catalog metadata.

Null, empty, invalid, or wraparound geometries do not participate in spatial
bucket pruning: their layout columns are null or assigned to a small overflow
bucket, and correctness falls back to the exact SedonaDB predicate.

## Automatic partition selection

The default mode is `AUTO`:

1. **Detect time.** Prefer explicit table options. Otherwise choose a timestamp
   column with common names such as `time`, `timestamp`, `datetime`,
   `observed_at`, `captured_at`, `acquired_at`, or `created_at`. Tables without
   time use a single `_qg_time_bucket = 'none'` bucket.
2. **Choose time granularity.** Pick hour/day/month/year buckets from the batch
   min/max and estimated row count, targeting large files and avoiding tiny
   buckets.
3. **Choose spatial mode.** Use WebMercator integer cells for SRID 4326/3857
   data. Use a table-local normalized Hilbert grid for arbitrary projected/CAD
   coordinates where WebMercator cells are meaningless. Prefer compact integer
   bucket IDs over string quadkeys in the physical files; expose readable names
   only in diagnostics.
4. **Cap partition fanout.** Never create a partition per feature. If a batch
   would exceed the configured open-partition budget, coarsen time first, then
   space.
5. **Write clustered files.** Sort each write batch by time bucket, space bucket,
   and Hilbert key before writing target-sized Parquet files. If a writer would
   open too many partitions, coarsen bucket resolution and record that choice in
   table layout metadata for deterministic future appends.

Initial defaults should be conservative: 512 MiB-1 GiB target files, row groups
around 128 MiB, and a per-writer open-partition cap low enough to avoid object
store and catalog pressure. Treat those as policy knobs, not SQL-visible index
definitions.

## Query pruning pipeline

For a predicate such as `ST_Intersects(geom, envelope)` plus an optional time
filter:

1. Extract the query envelope from recognized shapes: `&&`, `ST_Intersects`,
   `ST_Within`, `ST_Contains`, `ST_DWithin` with constant/parameter envelopes,
   `ST_MakeEnvelope`, `ST_TileEnvelope`, and GeoServer/QGIS bbox patterns.
2. Add hidden bbox overlap predicates:
   `_qg_minx <= query_maxx AND _qg_maxx >= query_minx AND _qg_miny <= query_maxy
   AND _qg_maxy >= query_miny`.
3. Derive candidate `_qg_space_bucket` values for coarse partition pruning.
4. Derive candidate `_qg_time_bucket` values for temporal partition pruning.
5. Let DuckLake/DataFusion prune partitions, files, and row groups with stats
   for the bucket, bbox, and time columns.
6. Reapply the original SedonaDB predicate exactly.

This gives a PostGIS-like spatial-index experience without a mutable GiST/R-tree
side structure.

Only add the rewrite when it is provably safe and selective. If the planner
cannot understand a spatial predicate shape, it leaves the query alone and still
gets correct SedonaDB results.

## LayoutBench validation dataset

Use a deterministic synthetic suite named **LayoutBench** for M5 validation. It
should be generated from a seed and scale factor so CI, developer machines, and
nightly stress runs exercise the same distributions at different sizes.

| Scale | Purpose | Approximate size |
|---|---|---:|
| `sf0` | CI smoke and exact-result oracle; implemented as `just layoutbench-sf0` | 252 rows today, grows as oracle cases are added |
| `sf1` | local developer benchmark | 1M-5M rows |
| `sf10` | nightly pruning/compaction benchmark | 10M-50M rows |
| `sf100+` | manual multi-writer/storage stress; 10 TB proxy | 100M+ rows / generated streaming |

The suite should create these tables:

| Table | Shape | What it validates |
|---|---|---|
| `layoutbench_aerial_frames` | Overlapping oriented photo footprints along flight strips, with `captured_at`, camera metadata, GSD, altitude, and mission id | area+time partitioning, many small polygons, high overlap, 10 TB aerial-capture ingest patterns |
| `layoutbench_cad_objects` | Dense local-coordinate building/site features: points, lines, polygons, floor/level, object type, source object id, Z range, tessellation tolerance | table-local Hilbert layout, CAD/BIM provenance, high coordinate precision, local engineering grids |
| `layoutbench_assets` | One row per large asset: COPC/LAZ/E57 point cloud, COG/GeoTIFF raster, 3D Tiles/glTF mesh, IFC/CityGML/LandXML/DXF-derived layer | asset-footprint indexing without exploding every point/triangle into SQL rows |
| `layoutbench_control_points` | Survey/control points observed at multiple acquisition epochs with known synthetic drift and vertical datum metadata | coordinate epoch, transform provenance, residual/accuracy checks over time |
| `layoutbench_queries` | Optional expected-result table for the `sf0` oracle windows | deterministic row counts for exact-vs-pruned validation |

The generator should deliberately include hard cases:

- mission/time skew: dense bursts in one hour plus sparse multi-year captures;
- spatial skew: downtown dense blocks, sparse rural areas, and long linear
  corridors;
- overlapping aerial frames with 70-85% overlap along flight strips;
- local CAD coordinates near large offsets, e.g. project grids with millimetre
  detail far from origin;
- Z/M/ZM geometries and sidecar Z ranges, while the M5 layout remains XY;
- invalid/empty/null geometries that must fail closed into exact predicate
  evaluation;
- source CRS changes and synthetic coordinate drift with known correction
  residuals.

Core benchmark queries:

1. **Tile/time window:** aerial frames intersecting a WebMercator tile and a one
   hour/day capture window.
2. **Mission strip:** all frames for a flight strip crossing several spatial
   buckets.
3. **CAD viewport:** objects on selected floors intersecting a local-coordinate
   viewport.
4. **Asset discovery:** point clouds/COGs/meshes intersecting an area/time window
   with resolution/accuracy filters.
5. **Drift residual:** control points transformed from acquisition epoch to a
   target epoch must stay within the synthetic accuracy threshold.
6. **Oracle equality:** for `sf0`, every layout-prefiltered query result must
   match the same exact SedonaDB predicate without layout-column pruning.
7. **Compaction:** append many small writer outputs, compact by bucket, then prove
   query results are unchanged while files/row groups scanned decrease.

Record these metrics for every run: ingest rows/sec, generated files, average
file and row-group size, DuckLake metadata rows, max open partitions per writer,
partition/file/row-group skip ratios, bytes scanned, exact-predicate candidate
false-positive ratio, wall-clock query time, compaction time, and coordinate
residual error.

Current `sf0` also asserts deterministic `_qg_*` bbox/bucket projection for
automatic hidden layout columns on new spatial tables, plus INSERT, UPDATE, and
transaction-staged UPDATE paths. Public `SELECT *` and client metadata hide those
columns; internal `quackgis.main.*` remains available for validation and future
planner rewrites. The first SQL-level pruning rewrite now injects `_qg_*` bbox
predicates for simple single-table `ST_Intersects(... ST_MakeEnvelope(...))` and
`&& ST_TileEnvelope(...)` query shapes while preserving the exact spatial
predicate. Its oracle counts are pinned by
`crates/quackgis-server/tests/layoutbench_sf0.rs`; the oracle also checks aliases,
derived single-table subqueries, wildcard projection safety, tile envelopes,
comment/string-safe clause scanning, and EXPLAIN visibility of injected layout
predicates:

```text
layoutbench_sf0 aerial=18 cad=12 assets=18 control=7
layoutbench_sf0_pruning aerial=108/30/18/18 cad=96/24/12/12 assets=24/20/18/18 false_positive=3/3/2/1
```

The pruning line is `total/base/candidate/exact`: all table rows, rows after
non-spatial predicates, rows after hidden bbox predicates, and rows after the
exact SedonaDB predicate. The local pgwire runner also emits `layoutbench_scan`
from `EXPLAIN ANALYZE`, including bytes scanned, row-group/file-range pruning,
Parquet pushdown rows, and whether `_qg_*` bbox predicates reached the physical
plan. The false-positive case intentionally includes a polygon hole whose bbox
overlaps the query window but whose exact geometry does not, pinning correctness
after over-selection.

## Parallel writes

Parallel writers should not coordinate through a global spatial index. Each
writer:

1. computes the same deterministic layout columns;
2. writes independent files under the chosen bucket paths;
3. commits through DuckLake snapshot metadata;
4. relies on optimistic conflict handling for table metadata, not per-row locks.

This keeps write amplification predictable for huge ingest jobs. If many writers
produce small files, QuackGIS compaction rewrites only the affected coarse
time/space buckets into larger sorted files.

For explicit transactions, keep the current single-table snapshot boundary:
staged writes compute the same hidden columns, then publish one final DuckLake
snapshot at commit.

## Bulk ingest and sf1 findings

The first pgwire `sf1` iteration uses a moderate local scale (`factor=100`,
22,800 rows total) to expose levers quickly before moving to nightly-sized data.
The results are in `benchmarks/BENCHMARKS.md`; the architecture implications are:

1. **COPY is the bulk ingest path.** Batched INSERT VALUES took about 16 s to seed
   the current sf1; pgwire `COPY ... FROM STDIN` took about 0.9 s with the same
   rows and correctness checks. QuackGIS therefore implements COPY IN as the path
   GDAL/OGR should use (`PG_USE_COPY=YES`) for `ogr2ogr`-style loads.
2. **Layout sorting must run at bulk granularity.** Sorting each tiny autocommit
   INSERT batch cannot recover locality when client row order is random: shuffled
   INSERT scanned most row groups. COPY and transaction-staged writes sort the
   whole table delta and prune back to one matched row group.
3. **Transaction grouping is a write-layout primitive.** Explicit transactions are
   not only semantic grouping; they give QuackGIS a staging boundary for sorting a
   table delta once before publishing it to DuckLake.
4. **Row groups are the current skip unit.** File/range pruning is not selective
   in current local sf1 runs; Parquet row-group statistics are. The local default
   `QUACKGIS_DUCKLAKE_ROW_GROUP_ROWS=512` is intentionally small for this scale
   and can be disabled with `0`. Larger/nightly scales should migrate this from a
   row-count cap toward a bytes/row-count policy aligned with DuckLake defaults.
5. **Compaction is the next architecture lever.** Many small autocommit append
   files should be rewritten into sorted bucket-local files. Correctness remains
   the exact SedonaDB predicate; compaction should only reduce files/ranges and
   row groups read.

The first implementation is an explicit table-scoped command:

```sql
CALL quackgis_compact_table('public.my_spatial_table');
```

It is intentionally boring: read the table through DataFusion, normalize/project
layout columns, sort by the hidden layout key, and rewrite the DuckLake table in
one replacement snapshot. This already repairs the bad shuffled/autocommit sf1
case (about 0.52 s to compact all three sf1 tables, cutting aerial from 22/18
matched row groups to 22/1 and files/ranges from 23/23 to 1/1). The next step is
to restrict this same primitive to changed coarse time/space buckets instead of
rewriting the whole table.

Current implemented path:

- CREATE/CTAS/INSERT/COPY writes add hidden `_qg_*` columns for spatial tables;
- the write path recomputes hidden values from WKB and ignores client-provided
  layout values;
- batches are sorted by `(_qg_time_bucket, _qg_space_bucket, _qg_space_sort)`
  before DuckLake writes;
- explicit transaction staging sorts the whole staged table at commit;
- COPY FROM STDIN parses PostgreSQL text COPY, including GDAL-style bytea/WKB
  octal/hex escapes, and routes through the same layout projection.
- `CALL quackgis_compact_table(...)` rewrites an existing table through the same
  projection/sort path to repair small-file or poorly ordered append layouts.

## Maintenance model

The low-maintenance path is:

- automatic hidden columns on create/insert/CTAS;
- automatic stats-driven pruning on read;
- optional `ANALYZE` to refresh table extent/time summaries used by `AUTO`;
- bucket-local compaction that preserves `(_qg_time_bucket, _qg_space_bucket,
  _qg_space_sort)` order;
- no user-managed partition DDL and no mutable global spatial index.

Manual table options remain an escape hatch for expert users:

- geometry column and SRID;
- time column and granularity;
- spatial grid mode/resolution;
- target file size and max open partitions;
- compaction policy.

## Implementation sequence

1. Add table layout metadata and hidden columns for new spatial tables. Mark WKB
   Arrow fields with `geoarrow.wkb` extension metadata where possible, but keep
   DuckLake storage as WKB Binary/GEOMETRY.
2. Add an internal batch layout projector in the DuckLake write path. It should
   parse each geometry once with Sedona `wkb_bounds_xy`, emitting bbox, space
   bucket, Hilbert sort key, and optional time columns.
3. Apply the projector to CTAS, INSERT, UPDATE rewrites, and transaction staging.
4. Sort write batches by time/space/sort key before handing them to the DuckLake
   writer API.
5. Add predicate rewrites for bbox/time pruning while preserving exact SedonaDB
   predicate evaluation.
6. Teach datafusion-ducklake, if needed, to expose enough partition/file stats to
   verify pruning decisions.
7. Implement LayoutBench `sf0` and assert exact-pruned equality plus deterministic
   counts in CI.
8. Add LayoutBench `sf1+` ingest/query/compaction benchmarks: aerial-like capture
   footprints, CAD-like local coordinates, asset-footprint rows, and control-point
   drift checks.

## Validation notes

Local GeoArrow probe command used during design:

```text
cargo run --quiet  # in .tmp/geoarrow_probe
geoarrow_type=Wkb(WkbType { metadata: Metadata { crs: Crs { crs: None, crs_type: None }, edges: None } })
geoarrow_len=1
geoarrow_first_point=(0,0)
```

The probe validates that Arrow `Binary` WKB plus `ARROW:extension:name =
geoarrow.wkb` is compatible with GeoArrow 0.8. It also confirms the path is still
serialized WKB row parsing, so M5 should optimize the existing WKB/Sedona path
rather than pivot to native GeoArrow arrays.
