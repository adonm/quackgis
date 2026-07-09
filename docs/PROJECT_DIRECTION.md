# Project direction

QuackGIS is a **PostGIS-compatible, Sedona-powered spatial lakehouse database**:
a single Rust pgwire server for platform and application teams that need to ask
large, complex spatial and columnar analytical questions across very large
datasets with high throughput.

The durable storage model is DuckLake: SQL catalog metadata plus Parquet data on
file/object storage. SQLite + local files and PostgreSQL + S3 are both first-class
storage profiles. The same SQL, pgwire, spatial layout, and compatibility surface
should work on both. SQLite/local must preserve the same semantics for local and
test deployments; scaled multi-writer/high-QPS gates target the PostgreSQL catalog
+ S3 object-storage profile.

## Primary user

The first-class user is a **platform or application developer** building spatial
services over a shared data lake:

- many stateless QuackGIS readers serving analytical/API traffic;
- many parallel ingest jobs, including GDAL/OGR/`ogr2ogr`-style tools;
- one shared DuckLake catalog and object-store data prefix;
- complex spatial SQL and OLAP-style aggregates over large tables without copying
  data into PostgreSQL or DuckDB.

Desktop/server GIS clients matter because they are the ecosystem interface. QGIS,
GDAL/OGR, GeoServer, Martin, `psql`, and psycopg should connect with minimal or no
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
5. hidden spatial layout columns, pruning, COPY ingest, and compaction for large
   table performance.

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

## Current preview, Alpha evidence, and broader direction

The current developer preview proves the shape locally:

1. PostGIS-compatible pgwire server in one Rust binary;
2. DuckLake writes on SQLite catalog + local Parquet files;
3. PostgreSQL text `COPY FROM STDIN` ingest for WKB geometry;
4. QGIS/GDAL/OGR/GeoServer/Martin compatibility smoke paths;
5. WKB-first hidden spatial layout and safe bbox pruning with exact SedonaDB
   recheck;
6. explicit compaction for fragmented append layouts.

The Alpha evidence loop now exists in Kind: PostgreSQL DuckLake catalog,
S3-compatible object storage, multiple QuackGIS pods, writer conflict/retry, QPS
reader probes, OLAP fanout probes, and uploaded compatibility/storage reports.
The next step is **Alpha hardening**: make that evidence credible enough for
external platform developers instead of treating a single green smoke as a
production claim.

- scheduled trend reports from `metrics.json` artifacts;
- larger/manual QPS and OLAP gates over LayoutBench and real OSM/Overture-derived
  layers;
- external PostgreSQL and S3-compatible services, not only in-cluster stand-ins;
- operational docs for catalog/object-store credentials, compaction, backup,
  failed-writer cleanup, and failure modes;
- production auth/RBAC/TLS and observability profiles.

Beyond Alpha, the project should aim at real-data client matrices, bucket-local
compaction/native write maintenance, temporal layout, snapshot time travel,
broader PostGIS conformance, and a 1.0 release that platform teams can operate
without reading the source tree.

## Scope boundaries

QuackGIS is not:

- a document database;
- an OLTP application database;
- a full PostgreSQL replacement;
- a general-purpose lakehouse engine unrelated to spatial workloads;
- a desktop GIS or map server.

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
  temporal layout/time travel, production security, and advanced analytics.
