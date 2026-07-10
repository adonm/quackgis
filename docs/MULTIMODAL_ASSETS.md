# Multi-modal spatial asset schemas

For benchmark promotion and real-data validation, use the asset-inventory query
families and scale ladder in [ANALYTICS_BENCHMARKS.md](./ANALYTICS_BENCHMARKS.md).

QuackGIS keeps the hot SQL path simple-feature first. Raster, point-cloud,
3D-tile, CAD/BIM, aerial, and reality-capture data enter the lakehouse as
queryable footprint/index rows plus sidecar URIs, not as heavyweight decoders in
the pgwire query path.

## Pattern

Use one DuckLake table per asset family with:

- stable collection, asset, source-object, and version identity for clients;
- a `geom` or `footprint` WKB `BINARY` column for PostGIS-compatible discovery;
- source-object metadata (`asset_uri`, `media_type`, `source_object_id`,
  `collection`, `captured_at`/bucket fields);
- quality/scale fields (`resolution_cm`, `gsd_cm`, `point_spacing_cm`,
  `accuracy_cm`, `tolerance_mm`, `z_min`, `z_max`);
- CRS/epoch/provenance fields (`srid`, `coordinate_epoch`, `vertical_datum`,
  `transform_pipeline`, `lineage_json`); and
- integrity/lifecycle fields such as checksum/etag, byte size, created/replaced
  snapshot, release/retention class, and source/derived relationship.

QuackGIS computes hidden layout columns from the WKB footprint at write time, so
the same spatial pruning/exact-recheck path works for asset inventories and for
ordinary vector layers.

## Starter DDL shapes

These are cheap schema-oracle examples, not promoted inventory schemas. Real
inventories must use a superset that includes the identity, version, checksum,
object-size, lifecycle/retention, and source/derived fields above.

```sql
CREATE TABLE public.raster_footprints (
  id BIGINT,
  collection TEXT,
  asset_uri TEXT,
  media_type TEXT,
  captured_minute INT,
  resolution_cm DOUBLE,
  horizontal_accuracy_cm DOUBLE,
  srid INT,
  coordinate_epoch DOUBLE,
  footprint BINARY
);

CREATE TABLE public.pointcloud_tiles (
  id BIGINT,
  collection TEXT,
  asset_uri TEXT,
  point_format TEXT,
  captured_minute INT,
  point_spacing_cm DOUBLE,
  z_min DOUBLE,
  z_max DOUBLE,
  srid INT,
  footprint BINARY
);

CREATE TABLE public.cad_bim_objects (
  id BIGINT,
  model_uri TEXT,
  source_object_id TEXT,
  object_type TEXT,
  floor INT,
  tolerance_mm DOUBLE,
  z_min DOUBLE,
  z_max DOUBLE,
  geom BINARY
);

CREATE TABLE public.reality_capture_frames (
  id BIGINT,
  capture_uri TEXT,
  camera_id TEXT,
  captured_minute INT,
  gsd_cm DOUBLE,
  altitude_m DOUBLE,
  transform_pipeline TEXT,
  footprint BINARY
);
```

## Query contract

Asset richness should augment, not destabilize, PostGIS client behavior:

```sql
SELECT id, asset_uri
FROM public.raster_footprints
WHERE captured_minute BETWEEN 1000 AND 2000
  AND ST_Intersects(
    ST_GeomFromWKB(footprint),
    ST_GeomFromWKB(ST_MakeEnvelope(-122.5, 37.7, -122.3, 37.9, 4326))
  );
```

Clients can discover these layers through `geometry_columns` when the footprint
column uses a conventional geometry name (`geom`, `footprint`, `shape`, or OGR/QGIS
variants that QuackGIS recognizes). Heavy source artifacts stay in object storage
and are fetched by application code only after SQL narrows the candidate set.

## Identity, lifecycle, and security

An asset URI is not a durable identity. Importers must preserve a stable logical
asset id across object rewrites and record the exact object version/checksum used
to derive the footprint. A release or protected snapshot must retain both the
index row and the referenced object version; deleting either independently breaks
reproducibility.

Object URIs may reveal bucket structure or grant access through embedded tokens.
QuackGIS catalog/API surfaces should therefore store stable non-secret locations,
filter asset rows with the same authorization as their collection, and leave
short-lived object authorization to the application/object-store boundary. SQL,
logs, metrics, and audit events must not expose signed URLs or credentials.

Derived geometry must name its source, transform pipeline, software/version,
tolerance, and acquisition/coordinate epoch. A future reprocessor should be able
to regenerate the query footprint and compare residuals without guessing how the
original conversion was performed.

## Current validation

`just layoutbench-sf0` is the cheap validation gate. It creates representative
aerial frame, CAD object, generic asset, and control-point tables, verifies hidden
layout values against sidecar bbox metadata, and proves bbox-prefiltered queries
return the same counts as exact SedonaDB predicates.

`just multimodal-inventory-local` adds a real-artifact companion gate using a
valid ESRI ASCII Grid/PRJ raster and valid ASCII PLY point cloud under
`tests/data/multimodal/`. `inventory-v1.json` pins their bytes, SHA-256, bounds,
CRS/epoch, vertical datum, transform provenance, quality, lifecycle, and stable
non-secret source URIs. The test:

1. parses raster and point-cloud headers/records without adding a server decoder;
2. recomputes checksum, byte size, XY/Z bounds, and validates the CRS sidecar;
3. detects a missing path and in-memory one-byte corruption;
4. rejects signed/credential-bearing source URIs at the fixture-import boundary;
5. writes full sidecar inventory rows through pgwire and verifies
   `geometry_columns` plus hidden footprint bounds;
6. proves exact-vs-hidden-bbox-pruned raster results and point-cloud quality/
   provenance filtering; and
7. supersedes one source-object version while retaining stable logical asset
   identity and both lifecycle rows.

This advances the deterministic gate beyond schema-only evidence, but it is not
COG, COPC/LAZ, object-store, regional, restore, or format-decoding support. The
first promoted real inventory still requires copied COG and COPC/LAZ collections
through the remaining acceptance ladder.

## Product acceptance ladder

Each asset family advances independently:

1. **Schema oracle:** deterministic footprint/layout/discovery checks (`sf0`).
2. **Real inventory:** ingest a copied collection and validate identity, checksums,
   CRS/epoch, footprint fidelity, missing/corrupt objects, and URI policy.
3. **Workload gate:** coverage/gap, quality filter, change-candidate, provenance,
   and mixed vector/asset queries with exact-result and scan budgets.
4. **Lifecycle gate:** replace/version/protect/release/restore an inventory while
   retaining the correct object versions and derived metadata.
5. **Scale gate:** record rows, object count/bytes, catalog size/refresh, query
   latency, files/row groups, and cost at regional then 10TB+ inventory scale.

The first promoted set should include one COG/raster collection and one COPC/LAZ
point-cloud collection. 3D Tiles, CAD/BIM, imagery, and reality capture follow the
same ladder rather than gaining support from a DDL example alone. Every promotion
must keep the maintained vector QGIS/GeoServer/GDAL gates green.
