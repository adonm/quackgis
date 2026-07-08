# Real OSM PostGIS parity roadmap

This track validates QuackGIS against real OpenStreetMap data while keeping
PostGIS as the reference source. It is intentionally **opt-in**: real OSM data is
downloaded into temporary runtime storage and is not committed to this
repository.

## Why this track matters

Synthetic point probes are good for fast compatibility triage, but they do not
exercise realistic geometry distributions, attribute names, NULL density, mixed
feature classes, client metadata paths, or practical data-copy workflows. The
goal here is to make this workflow boring:

1. Load a real OSM extract into PostGIS.
2. Copy selected layers into QuackGIS.
3. Open and compare both databases with OGR, QGIS, GeoServer, and Martin.
4. Document repeatable one-shot refresh and incremental-copy patterns.

## Data policy

- Default extract: Geofabrik Monaco, `monaco-latest.osm.pbf`, small enough for a
  local Kind loop while still being real OSM data.
- Data is downloaded at probe time into container-local storage.
- Do not commit OSM extracts or derived datasets.
- OSM data is licensed under ODbL. Any published derived data or screenshots
  need appropriate OpenStreetMap attribution and ODbL compliance review.

## Long roadmap

### Phase 0 — opt-in real-data plumbing

Status: the opt-in gate covers Monaco `points`, `lines`, and `multipolygons`.
It asserts deterministic count, stable IDs, `osm_id`, UTF-8 names, geometry type
distribution, and bbox through GeoJSON exports, then repeats the
`id`/`osm_id`/`name` comparison through SQL.

- Add a Kind PostGIS reference deployment in the `quackgis` namespace.
- Add an opt-in `just kind-osm-postgis-parity` target, separate from the default
  fast `just kind-probes` loop.
- Download Monaco `.osm.pbf` at runtime.
- Load real OSM points, lines, and multipolygons into PostGIS with GDAL/OGR.
- Create small deterministic samples from the real PostGIS tables.
- Pre-create equivalent WKB-backed QuackGIS tables.
- Copy PostGIS → QuackGIS with `ogr2ogr` using `PG_USE_COPY=NO` and
  `-addfields` so real OSM attributes such as `osm_id` are added by the same OGR
  path users will run.
- Compare counts, IDs, geometry types, and bbox through GeoJSON exports from
  both databases, then compare `id`/`osm_id`/UTF-8 `name` rows through SQL.

### Phase 1 — multi-layer OSM copy parity

Status: implemented for OGR's standard OSM `points`, `lines`, and
`multipolygons` layers.

The current probe copies these standard OGR OSM layers:

- `points` → `osm_points` / `wkb_geometry` Point.
- `lines` → `osm_lines` LineString/MultiLineString-compatible WKB.
- `multipolygons` → `osm_multipolygons` Polygon/MultiPolygon-compatible WKB.
- Samples are deterministic: stable `ORDER BY`, explicit limits, explicit table
  names, and printed counts.
- The gate compares:
  - feature count;
  - selected attributes (`osm_id`, `name`, class tags where available);
  - geometry type distribution;
  - bbox.

Expected compatibility gaps to flush out:

- GDAL layer creation defaults that assume real PostgreSQL DDL/type support.
- Geometry type promotion (`POLYGON` vs `MULTIPOLYGON`, line variants).
- Attribute type mapping and long `other_tags` strings.
- Schema-derived OGR metadata for arbitrary appended columns. The maintained OGR
  probe now fails if an appended field is missing from GeoJSON export; repeat
  this coverage as QGIS/GeoServer OSM-layer probes are added.
- Keep UTF-8 text handling under real-data regression. Phase 0 now verifies
  Monaco names such as `Quai des États-Unis` and `La Pêcherie U Luvassu`; later
  phases should repeat that coverage across all copied OSM layers.
- Large COPY loads over pgwire against pre-created WKB-backed schemas. The
  maintained OSM parity probe still uses `PG_USE_COPY=NO` for schema-evolving
  `-addfields` append coverage; COPY is now available for focused bulk-load
  probes.

### Phase 2 — side-by-side client access matrix

For each copied OSM layer, open both the PostGIS source and QuackGIS copy with:

| Client | PostGIS source | QuackGIS copy | Assertions |
|---|---:|---:|---|
| OGR | ✅ | ✅ | `ogrinfo`, GeoJSON export, count/bbox/type parity |
| QGIS | ✅ | ✅ | provider validity, fields, feature iteration, render smoke |
| GeoServer | ✅ | ✅ | datastore publish, WFS count, WMS PNG |
| Martin | ✅ | ✅ | discovery and non-empty MVT tile |

This phase should produce an explicit compatibility report per layer rather than
claiming broad OSM support from a single points table.

### Phase 3 — documented copy workflows

Document practical recipes people can use immediately. For schema-evolving
OGR append flows, keep `PG_USE_COPY=NO` until that probe is intentionally moved;
for pre-created WKB-backed tables, PostgreSQL text COPY is now a supported bulk
ingest path.

#### One-shot copy of a source table

```sh
PG_USE_COPY=NO ogr2ogr \
  -f PostgreSQL "PG:host=quackgis port=5434 user=postgres dbname=quackgis" \
  "PG:host=postgis port=5432 user=postgres dbname=osm" \
  -sql "SELECT id, osm_id, name, geom FROM public.osm_points" \
  -nln osm_points -append -nlt POINT -lco GEOMETRY_NAME=wkb_geometry
```

#### Overwrite/refresh pattern

Until QuackGIS has a fast native bulk-replace path, prefer an explicit staging
table:

1. Create `osm_points_next` in QuackGIS.
2. Load/copy all rows into `osm_points_next`.
3. Validate counts/bbox/sample attributes.
4. Swap names or drop/recreate the published table during a maintenance window.

#### Incremental append pattern

For upstream tables with a reliable change column:

```sh
PG_USE_COPY=NO ogr2ogr \
  -f PostgreSQL "PG:host=quackgis port=5434 user=postgres dbname=quackgis" \
  "PG:host=postgis port=5432 user=postgres dbname=osm" \
  -sql "SELECT id, osm_id, name, geom FROM public.osm_points WHERE updated_at > TIMESTAMP '2026-01-01'" \
  -nln osm_points -append -nlt POINT -lco GEOMETRY_NAME=wkb_geometry
```

For raw OSM, true minutely replication belongs in the PostGIS/osm2pgsql side
first; QuackGIS should initially consume curated changed tables from PostGIS,
not implement OSM replication itself.

### Phase 4 — write/edit workflows against OSM-derived layers

- GeoServer WFS-T insert/update/delete against a QuackGIS OSM-derived layer.
- QGIS edits against copied OSM layers with real-world attribute widths and NULLs.
- Conflict tests for refresh-while-client-open.

This is where general pgjdbc fetch-size portal suspension, geometry write
parameters, and privilege metadata should be implemented if traces require them.

### Phase 5 — performance and larger extracts

Move beyond Monaco only after correctness is boring:

- Andorra / Liechtenstein / Isle of Man extracts.
- Count/bbox/type parity under larger row counts.
- Insert throughput tracking with and without COPY support.
- Martin tile latency and GeoServer WMS render latency.
- DuckLake file counts, Parquet sizes, and read pruning behavior.
- OLAP fanout queries over OSM-derived layers: grouped counts/lengths/areas by
  class/tag, calculated filters for candidate records, and exact spatial recheck
  after pruning/pushdown.

This phase should decide whether the next highest-value storage feature is:

- PostgreSQL/S3 Alpha storage hardening for larger shared OSM copies;
- a high-QPS parallel-reader probe over copied OSM-derived layers;
- an OLAP fanout benchmark over OSM-derived columns and geometries;
- bucket-local compaction for append-heavy OSM refreshes;
- or a QuackGIS-native bulk load path beyond PostgreSQL text COPY.

### Phase 6 — production sync guidance

Document recommended architectures:

1. **PostGIS as OSM ingest/cache, QuackGIS as analytical/read-serving copy.**
   Use osm2pgsql or imposm into PostGIS, curate SQL views/tables, then copy into
   QuackGIS on a schedule.
2. **Snapshot refresh.** Nightly full extract into staging, validate, swap.
3. **Delta tables.** PostGIS computes inserts/updates/deletes into explicit
   change tables; QuackGIS consumes those changes with deterministic DML.
4. **No unsupported claims.** Logical replication, triggers, and full PostgreSQL
   extension semantics remain non-goals unless explicitly implemented later.

## First implemented gate

The first gate is intentionally small but real:

```sh
eval "$(mise activate bash)"
just kind-refresh-fast
just kind-osm-postgis-parity
```

Expected high-level output:

```text
osm_extract_url https://download.geofabrik.de/europe/monaco-latest.osm.pbf
postgis_osm_named_points_count <n>
quackgis_osm_named_points_count <n>
postgis_osm_sql_sample [...]
quackgis_osm_sql_sample [...]
osm_postgis_to_quackgis_copy_ok True
```

This proves real OSM → PostGIS → QuackGIS copy/read parity for deterministic
Point, LineString, and MultiPolygon-compatible samples across stable IDs, OSM
IDs, UTF-8 names, geometry type distribution, and bbox. Later phases widen
client coverage beyond OGR.
