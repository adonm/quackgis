# Project direction

This is the stable product charter. It intentionally does not repeat current
status or milestone detail:

- [ROADMAP.md](../ROADMAP.md) owns future outcomes and exit gates;
- [ARCHITECTURE.md](../ARCHITECTURE.md) owns design and invariants; and
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md) owns the implemented evidence floor.

## Product thesis

QuackGIS is the **PostGIS-compatible SQL and control plane for an open spatial
lakehouse**. It lets platform and application teams keep large shared spatial data
in DuckLake/Parquet, execute through DataFusion + SedonaDB, and preserve the GIS,
driver, API, and BI ecosystem that already understands PostgreSQL/PostGIS.

The durable advantage is the combination, not any one subsystem:

1. familiar pgwire/PostGIS workflows at the edge;
2. exact vectorized spatial and columnar analysis in the engine;
3. snapshot-based open lake storage for scale and recoverability;
4. trace-driven compatibility rather than speculative PostgreSQL emulation; and
5. dataset release, maintenance, metadata, and evidence surfaces that make the
   lake operable.

## Primary user and job

The first-class user is a platform team running shared spatial services:

- stateless readers serving GIS, APIs, tiles, and analytical traffic;
- parallel GDAL/OGR and application ingest/edit jobs;
- managed catalog/object storage;
- real spatial SQL and grouped/window/join analysis where supported; and
- release, restore, retention, and upgrade workflows over large datasets.

The core job is:

> Answer large, complex spatial questions over a shared lake with horizontal read
> scale and columnar analytics while preserving common PostGIS client workflows
> and exact spatial results.

## Product horizons

- **1.0:** an operational regional vector lakehouse with managed-service,
  real-data client, security, recovery, upgrade, and budgeted scale evidence.
- **1.x:** releaseable datasets, protected history, staged promotion/rollback, and
  maintained tile/coverage summaries.
- **2.x:** a multi-modal SQL/control plane for raster, point-cloud, 3D, CAD/BIM,
  imagery, and reality-capture inventories, with national/trillion-class stress
  evidence and measured sharding limits.

## Scope discipline

QuackGIS is not a smaller PostgreSQL, OLTP database, desktop GIS, map server, or
universal spatial-format decoder. PostgreSQL is an interface and optional catalog
store; DuckDB is the unreleased storage-authority target but not yet embedded in
the current runtime; heavy assets remain in object storage behind queryable
footprint/provenance indexes.

Compatibility breadth is earned from maintained workflows. Storage-specific
features are adopted from DuckLake when they preserve QuackGIS correctness and
interoperability gates. When PostgreSQL illusion, optimization, and correctness
conflict, explicit limits and correctness win.

## Claim discipline

Every claim names its evidence ring:

- local deterministic semantics;
- Kind/multi-pod integration;
- managed-service provider behavior;
- copied real-data client/product behavior; or
- release-grade upgrade, recovery, and soak evidence.

Plans and schemas are design contracts. They become product claims only when the
matching workload runs at the stated scale and source SHA with reviewable
artifacts.
