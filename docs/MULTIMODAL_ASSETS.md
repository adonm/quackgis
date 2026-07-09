# Multi-modal spatial asset schemas

For benchmark promotion and real-data validation, use the asset-inventory query
families and scale ladder in [ANALYTICS_BENCHMARKS.md](./ANALYTICS_BENCHMARKS.md).

QuackGIS keeps the hot SQL path simple-feature first. Raster, point-cloud,
3D-tile, CAD/BIM, aerial, and reality-capture data enter the lakehouse as
queryable footprint/index rows plus sidecar URIs, not as heavyweight decoders in
the pgwire query path.

## Pattern

Use one DuckLake table per asset family with:

- a stable integer/string identity column for clients;
- a `geom` or `footprint` WKB `BINARY` column for PostGIS-compatible discovery;
- source-object metadata (`asset_uri`, `media_type`, `source_object_id`,
  `collection`, `captured_at`/bucket fields);
- quality/scale fields (`resolution_cm`, `gsd_cm`, `point_spacing_cm`,
  `accuracy_cm`, `tolerance_mm`, `z_min`, `z_max`);
- CRS/epoch/provenance fields (`srid`, `coordinate_epoch`, `vertical_datum`,
  `transform_pipeline`, `lineage_json`).

QuackGIS computes hidden layout columns from the WKB footprint at write time, so
the same spatial pruning/exact-recheck path works for asset inventories and for
ordinary vector layers.

## Starter DDL shapes

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

## Current validation

`just layoutbench-sf0` is the cheap validation gate. It creates representative
aerial frame, CAD object, generic asset, and control-point tables, verifies hidden
layout values against sidecar bbox metadata, and proves bbox-prefiltered queries
return the same counts as exact SedonaDB predicates.
