# Real-data client matrix

Synthetic fixtures keep compatibility cheap. The real-data matrix is the next
step: copy representative external datasets into QuackGIS, compare them against a
PostGIS reference where useful, and exercise the same layers through maintained
clients.

This document defines the evidence contract before widening the expensive probes.

## Current implemented baseline

| Dataset | Layers | Clients exercised today | Evidence |
|---|---|---|---|
| Geofabrik Monaco OSM | `points`, `lines`, `multipolygons` sampled through OGR's standard OSM layers | OGR copy/read, QuackGIS SQL sample comparison, QGIS open/filter/render on copied layers | `just kind-osm-postgis-parity`, `docs/OSM_POSTGIS_PARITY.md` |

Current status is **opt-in real-data evidence**, not broad OSM or real-data
support. The default scheduled compatibility workflow runs the Monaco path because
it is small enough for CI; larger extracts remain manual until budgets are stable.

## Matrix target

Every promoted real-data matrix row should name:

- source dataset and license/attribution requirements;
- exact extract URL or immutable object-store prefix;
- source row counts and copied row counts;
- geometry columns, geometry type distribution, SRID/CRS assumptions, and bbox;
- attribute width/NULL/UTF-8 coverage notes;
- client versions from `docs/COMPATIBILITY_MATRIX.md`;
- metrics artifact path and dashboard summary;
- unsupported surfaces or skipped client workflows.

## Required client assertions

| Client/workflow | Minimum assertions before support claim |
|---|---|
| OGR/GDAL copy/read | source → QuackGIS copy succeeds; `ogrinfo` reports expected layer schema; GeoJSON export count/bbox/type parity; appended fields survive schema-derived metadata |
| QGIS read/render/filter | provider opens; feature count is non-zero and matches sampled expectation; attribute filter returns deterministic names/ids; geometry iteration is non-empty; headless render writes an image |
| GeoServer WFS/WMS | datastore publishes the copied layer; WFS GeoJSON count/attributes match sample; WMS returns a PNG; paging behavior is recorded when enabled |
| GeoServer WFS-T/edit | insert/update/delete round-trips on a copied or derivative layer; `_quackgis_rowid` or declared primary key remains stable after compaction |
| Martin/MVT | table discovery includes the layer; a representative tile is non-empty; important feature attributes survive into tile metadata when configured |
| SQL/OLAP | representative grouped counts/lengths/areas and candidate-narrowing queries emit row counts, file groups, bytes scanned, and p95/p99 where applicable |

Python/API/BI-style clients follow the broader probe ladder in
[API_CLIENT_PROBES.md](./API_CLIENT_PROBES.md) before they become maintained
real-data matrix rows.

## Dataset ladder

| Phase | Dataset/profile | Purpose | Promotion gate |
|---|---|---|---|
| 0 | Monaco OSM, sampled | small real-data correctness and UTF-8/attribute smoke | current `just kind-osm-postgis-parity` |
| 1 | Monaco OSM, all copied standard layers | same geography, less sampling bias | OGR/QGIS/GeoServer/Martin row in metrics report |
| 2 | Andorra/Liechtenstein/Isle of Man OSM | modest row-count growth on real OSM | file/row-count budgets plus QGIS/GeoServer/Martin matrix |
| 3 | Overture/GeoParquet-derived wide attributes | wide columns, mixed geometry fields, NULL density | copied layers with schema parity and OLAP fanout dashboard |
| 4 | Manual regional extract | larger object-store layout and compaction behavior | external PostgreSQL/S3 evidence packet with dashboard |

Do not promote a larger phase if the smaller phase still has undocumented client
or schema deltas.

## Artifact shape

Real-data probes should add one `checks.<name>.metrics` block per dataset/client
combination where practical. Preferred metric keys:

```json
{
  "dataset": "osm-monaco",
  "layer": "osm_points",
  "source_rows": 1000,
  "quackgis_rows": 1000,
  "feature_count": 1000,
  "bbox": "BOX(...)",
  "geometry_types": {"POINT": 1000},
  "qps": 12.3,
  "p95_ms": 42.0,
  "p99_ms": 80.0,
  "bytes_scanned": 123456,
  "file_groups": 2
}
```

`scripts/trend_metrics.py` already extracts common QPS, latency, scan, native DML,
and OSM feature-count fields. Add new columns only when a dashboard consumer needs
the value across runs; otherwise keep dataset-specific detail in the rendered
compatibility report.

## Copy workflow guardrails

- Keep raw external data out of the repository.
- Store downloaded extracts and generated layers under `.tmp/` or external object
  prefixes created for the run.
- Record source license/attribution in the report before publishing screenshots,
  exported features, or derived datasets.
- Prefer PostGIS as the real-data ingest/cache reference for OSM replication or
  source-specific importers; QuackGIS consumes curated copied tables.
- For schema-evolving `ogr2ogr -append -addfields` flows, keep the existing OGR
  path until a COPY-based bulk-load matrix proves equal schema behavior.
- For large external-service phases, follow
  [ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md) before making scale or
  durability claims.
- For QPS, OLAP, scan-budget, and compaction evidence, follow
  [ANALYTICS_BENCHMARKS.md](./ANALYTICS_BENCHMARKS.md).

## Completion rule

A real-data matrix item is complete only when the docs name the dataset, clients,
commands, metric artifacts, pass/fail budget, and unsupported skips. A successful
one-off manual run without artifacts is useful debugging evidence, not a support
claim.
