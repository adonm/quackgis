# Project direction

QuackGIS is a **PostGIS-compatible front door to a spatial lakehouse**: a single
Rust pgwire server for platform and application teams that need to keep very
large spatial data in open DuckLake/Parquet storage while serving familiar
PostGIS clients and high-throughput analytical SQL.

The durable storage model is DuckLake: SQL catalog metadata plus Parquet data on
file/object storage. SQLite + local files and PostgreSQL + S3 are both first-class
storage profiles. The same SQL, pgwire, spatial layout, and compatibility surface
should work on both. SQLite/local must preserve the same semantics for local and
test deployments; scaled multi-writer/high-QPS gates target the PostgreSQL catalog
+ S3 object-storage profile.

QuackGIS should stay deliberately aligned with DuckLake upstream. Fork-backed
storage semantics are acceptable to unblock spatial workflows, but stable
DuckLake features should replace QuackGIS-only workarounds: deletion-vector/Puffin
improvements for DML, protected snapshots and future branch/merge for operations,
materialized views for maintained spatial summaries, VARIANT/UDT/fixed-size-array
support for asset metadata, and PostgreSQL catalog roundtrip/metadata-scan
optimizations for scale.

## Primary user

The first-class user is a **platform or application developer** building spatial
services over a shared data lake:

- many stateless QuackGIS readers serving analytical/API traffic;
- many parallel ingest jobs, including GDAL/OGR/`ogr2ogr`-style tools;
- one shared DuckLake catalog and object-store data prefix;
- complex spatial SQL and OLAP-style aggregates over large tables without copying
  data into PostgreSQL or DuckDB.

Desktop/server GIS clients matter because they are the ecosystem interface. QGIS,
GDAL/OGR, GeoServer, Martin, `psql`, psycopg, SQLAlchemy/GeoPandas,
pg_featureserv-style services, and BI tools should connect with minimal or no
client changes, but QuackGIS is not primarily a desktop GIS product.

## Primary job

The core job is:

> Answer large, complex spatial questions over a big shared dataset with high
> performance and horizontal read scaling, then analyze the filtered columnar
> records with OLAP-style aggregations/calculations, while remaining compatible
> with common PostGIS client workflows.

This drives the architecture:

1. pgwire + PostGIS-compatible catalog/SQL surface for tools;
2. SedonaDB for spatial execution and exact predicate correctness;
3. DataFusion-style vectorized SQL for columnar projections, aggregates, joins,
   expressions, and filter pushdown;
4. DuckLake/Parquet for lakehouse storage, snapshots, parallel readers, and object
   storage scale;
5. hidden spatial/temporal layout columns, pruning, COPY ingest, native DML, and
   compaction for large-table performance;
6. operational metadata, trendable metrics, and backup/restore evidence as first-
   class product surfaces.

“DuckDB-style OLAP” is a target user experience: fast ad hoc analytical SQL over
column-oriented data, with primitive aggregations/calculations pushed close to the
Parquet scan where possible. It does **not** mean embedding DuckDB.

Example target workload:

1. fan out across many geometry rows/assets;
2. compute spatial stats such as bbox/area/length/intersection counts or grouped
   coverage metrics;
3. combine those with primitive columnar aggregates such as `count`, `sum`,
   `avg`, percentiles/histograms where available, and conditional expressions;
4. use the aggregate/calculated result to filter relevant records for follow-up
   exact SedonaDB spatial predicates or downstream clients.

Longer term, QuackGIS should also index high-fidelity spatial assets: raster
mosaics, point-cloud tiles, 3D tiles, CAD/BIM objects, aerial/reality-capture
frames, and provenance sidecars. The SQL hot path should query footprints,
quality/resolution fields, CRS/epoch metadata, lineage, and storage URIs while
leaving heavyweight source artifacts in object storage.

## Current preview, Alpha evidence, and broader direction

The current developer preview/Alpha base proves the core shape:

1. PostGIS-compatible pgwire server in one Rust binary;
2. DuckLake writes on SQLite/local and Alpha PostgreSQL/S3 profiles;
3. PostgreSQL text `COPY FROM STDIN`, CTAS/INSERT/UPDATE/DELETE, native
   autocommit delete/update, and bucket-local compaction paths;
4. QGIS/GDAL/OGR/GeoServer/Martin compatibility smoke paths;
5. WKB-first hidden spatial layout, safe bbox pruning, temporal `BETWEEN` bucket
   prefilters, and exact SedonaDB recheck;
6. SCRAM password mode, coarse read/write vs read-only roles, explicit-user
   privilege metadata, DuckLake metadata UDTFs, local backup/restore oracle,
   trendable `metrics.json` artifacts plus Markdown dashboards, and an opt-in
   Prometheus metrics endpoint.

The Alpha evidence loop now exists in Kind: PostgreSQL DuckLake catalog,
S3-compatible object storage, multiple QuackGIS pods, writer conflict/retry,
native DML/compaction metadata probes, QPS reader probes, OLAP fanout probes,
metrics scraping, and uploaded compatibility/storage reports. The next step is
**Alpha hardening**:
make that evidence credible enough for external platform developers instead of
treating in-cluster stand-ins as production claims.

- release-attached trend dashboards from scheduled/manual `metrics.json` artifacts;
- larger/manual QPS and OLAP gates over LayoutBench, real OSM, and
  Overture-derived layers;
- external PostgreSQL and S3-compatible services, not only in-cluster stand-ins;
- operational probes/docs for catalog/object-store credentials, compaction,
  backup/restore, failed-writer cleanup, catalog refresh, and failure modes;
- production auth/RBAC/TLS hardening, richer metrics, and Kubernetes deployment
  profiles beyond the current reviewed Alpha example.

Beyond Alpha, the project should aim at real-data client matrices, native write
maintenance, temporal layout, SQL time travel, broader PostGIS conformance,
multi-modal asset sidecar schemas, and a 1.0 release that platform teams can
operate without reading the source tree.

## Scope boundaries

QuackGIS is not:

- a document database;
- an OLTP application database;
- a full PostgreSQL replacement;
- a general-purpose lakehouse engine unrelated to spatial workloads;
- a desktop GIS or map server;
- a heavyweight raster/CAD/point-cloud decoder in the SQL hot path.

QuackGIS does include columnar OLAP analysis because spatial lakehouse workloads
need it: fanout scans, grouped stats, primitive calculations, and pushdown filters
over Parquet columns are part of the target direction when they support spatial
analysis.

QuackGIS should still emulate enough transactional, catalog, and protocol behavior
to avoid client changes for common PostGIS/GIS workflows. When correctness and
PostgreSQL illusion conflict, correctness and explicit limits win.

## Claim style

Docs should distinguish:

- **current preview claims**: only features covered by tests/smokes/benchmarks;
- **alpha evidence**: PostgreSQL + S3, multi-process readers/writers, high-QPS
  scaled gates, OLAP gates, and trendable metrics;
- **alpha hardening**: real external services, larger scheduled datasets, and ops
  docs before production claims;
- **later roadmap**: broader real-data compatibility, native write maintenance,
  temporal layout/time travel, production security, advanced analytics, and
  multi-modal spatial asset indexing.
