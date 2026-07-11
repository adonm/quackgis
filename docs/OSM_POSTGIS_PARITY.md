# Real OSM PostGIS parity roadmap

This track validates QuackGIS against real OpenStreetMap data while keeping
PostGIS as the reference source. It is intentionally **opt-in**: real OSM data is
downloaded into temporary runtime storage and is not committed to this
repository.

See [REAL_DATA_CLIENT_MATRIX.md](./REAL_DATA_CLIENT_MATRIX.md) for the broader
dataset/client evidence contract that this OSM track feeds.

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

## Implemented Monaco baseline

The opt-in `just kind-osm-postgis-parity` gate deploys a PostGIS reference,
downloads Monaco at runtime, loads OGR's standard OSM layers, creates deterministic
samples, copies them into pre-created WKB-backed QuackGIS tables with `ogr2ogr`,
and compares the two systems. It covers:

- `points` → `osm_points` / `wkb_geometry` Point.
- `lines` → `osm_lines` LineString/MultiLineString-compatible WKB.
- `multipolygons` → `osm_multipolygons` Polygon/MultiPolygon-compatible WKB.
- deterministic samples: stable `ORDER BY`, explicit limits, explicit table
  names, and printed counts.
- count, stable id/`osm_id`, selected attributes, UTF-8 names, geometry type
  distribution, and bbox through SQL/GeoJSON;
- non-empty MVT SQL bytes plus real `name` attribute tokens for each copied layer; and
- QGIS provider validity, feature iteration/filtering, non-empty geometries, and
  a rendered image over the QuackGIS copies.

## Remaining OSM promotion work

- QGIS PostGIS-source side-by-side over real OSM layers; the current gate opens,
  filters, iterates, and renders the copied QuackGIS layers.
- Run GeoServer WFS/WMS/WFS-T on the copied OSM layers; generic GeoServer fixtures
  do not close this real-data row.
- Run the real Martin binary and verify configured OSM attributes, not only the
  SQL-MVT real-attribute token companion gate.
- Keep geometry promotion, long `other_tags`, arbitrary appended columns, and
  UTF-8 text under real-data regression. The baseline verifies
  Monaco names such as `Quai des États-Unis` and `La Pêcherie U Luvassu`; later
  datasets should repeat that coverage.
- Add large COPY loads over pgwire against pre-created WKB-backed schemas. The
  maintained OSM parity probe still uses `PG_USE_COPY=NO` for schema-evolving
  `-addfields` append coverage; COPY is now available for focused bulk-load
  probes.
- Promote to wider extracts and the versioned city client matrix in
  [REAL_DATA_CLIENT_MATRIX.md](./REAL_DATA_CLIENT_MATRIX.md).

### Side-by-side client access matrix

For each copied OSM layer, open both the PostGIS source and QuackGIS copy with:

| Client | PostGIS source | QuackGIS copy | Assertions |
|---|---:|---:|---|
| OGR | ✅ | ✅ | `ogrinfo`, GeoJSON export, count/bbox/type parity |
| QGIS | stretch | ✅ | provider validity, feature iteration/filtering, and render smoke for QuackGIS copy; PostGIS side-by-side remains next |
| GeoServer | ⏳ | ⏳ | generic datastore/WFS/WMS/WFS-T gates pass; copied OSM side-by-side is not yet part of this track |
| Martin/MVT | ⏳ | SQL attribute tokens | non-empty MVT bytes and `name` tokens for copied QuackGIS layers; real Martin binary attribute propagation remains next |

This phase should produce an explicit compatibility report per layer following
the real-data matrix contract rather than claiming broad OSM support from a
single points table.

### Documented copy workflows

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

### Write/edit workflows against OSM-derived layers

- GeoServer WFS-T insert/update/delete against a QuackGIS OSM-derived layer.
- QGIS edits against copied OSM layers with real-world attribute widths and NULLs.
- Conflict tests for refresh-while-client-open.

This is where the implemented pgjdbc fetch-size portal suspension path should be
exercised at realistic page sizes. Expand geometry write parameters and privilege
metadata only when traces require them.

### Performance and larger extracts

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

- managed-service PostgreSQL/S3 evidence for larger shared OSM copies;
- high-QPS and OLAP gates over copied OSM-derived layers rather than only
  synthetic tables;
- real edit-history and bucket-compaction evidence for append-heavy refreshes;
- or a QuackGIS-native bulk load path beyond PostgreSQL text COPY if COPY becomes
  the measured bottleneck.

### Production sync guidance

Document recommended architectures:

1. **PostGIS as OSM ingest/cache, QuackGIS as analytical/read-serving copy.**
   Use osm2pgsql or imposm into PostGIS, curate SQL views/tables, then copy into
   QuackGIS on a schedule.
2. **Snapshot refresh.** Nightly full extract into staging, validate, swap.
3. **Delta tables.** PostGIS computes inserts/updates/deletes into explicit
   change tables; QuackGIS consumes those changes with deterministic DML.
4. **No unsupported claims.** Logical replication, triggers, and full PostgreSQL
   extension semantics remain non-goals unless explicitly implemented later.

## Baseline gate command

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
osm_mvt_points_attribute_ok True
osm_mvt_lines_attribute_ok True
osm_mvt_multipolygons_attribute_ok True
osm_postgis_to_quackgis_copy_ok True
```

This proves real OSM → PostGIS → QuackGIS copy/read parity for deterministic
Point, LineString, and MultiPolygon-compatible samples across stable IDs, OSM
IDs, UTF-8 names, geometry type distribution, bbox, and SQL-MVT `name` attribute
tokens. Remaining promotion widens the existing OGR/QGIS/SQL-MVT baseline to
PostGIS-side QGIS, copied-layer GeoServer, real Martin attributes, larger
extracts, and managed storage.
