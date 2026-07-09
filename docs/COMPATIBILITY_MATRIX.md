# Compatibility matrix

QuackGIS compatibility is trace-driven. A row is supported only when a maintained
probe or focused regression covers the workflow named in the evidence column.

| Surface | Version/profile | Status | Evidence |
|---|---|---|---|
| PostgreSQL wire / `psql` | PostgreSQL protocol v3, server-version compatibility string `16.0` | ✅ maintained | `just smoke`, `wire_spatial` integration tests |
| psycopg / tokio-postgres style extended protocol | pure pgwire clients; text + binary params/results | ✅ maintained | `wire_spatial`, `ducklake_persistence`, `postgis_regress` |
| QGIS PostgreSQL provider | `docker.io/qgis/qgis:ltr-questing` | ✅ maintained read + edit | `just kind-qgis-probe`, `just kind-qgis-edit-probe`, `just kind-compatibility` |
| GDAL/OGR PostgreSQL driver | QGIS LTR image GDAL/OGR stack | ✅ maintained read/load | `just kind-ogr-probe`; OSM parity uses `ogr2ogr` opt-in |
| GeoServer PostGIS datastore | `docker.osgeo.org/geoserver:3.0.0` + pgjdbc `42.7.11` | ✅ maintained WFS/WMS/WFS-T | `just kind-geoserver-probe` |
| Martin | `martin` `1.11.0` | ✅ maintained SQL fixtures; real binary opt-in | `just martin-sql` includes MVT attribute tags, `just martin-e2e` |
| PostGIS SQL function surface | starter curated pgwire subset plus documented portability fixtures | ✅ starter pass-rate; broader ledger | `just postgis-regress`, weekly `PostGIS regress subset` workflow, `docs/POSTGIS_CONFORMANCE.md` |
| DuckLake SQLite/local profile | SQLite catalog + local Parquet files | ✅ maintained correctness | `just check-fast`, `just preview-smoke`, persistence tests |
| DuckLake PostgreSQL/S3 profile | PostgreSQL 16 + S3-compatible object store (`s3s-fs` in Kind) | ✅ Alpha evidence; not production durability | `just kind-alpha-smoke` |
| External PostgreSQL/S3 Alpha promotion | real platform-managed PostgreSQL + S3-compatible storage | ⏳ runbook ready; execution required | `docs/ALPHA_EXTERNAL_SERVICES.md`, `just kind-external-alpha-smoke` |
| Snapshot-pinned reads | simple DuckLake one-table snapshot selector with count/extent parity | ✅ local pgwire surface | `public.table(snapshot => <id>)`, `ducklake_snapshot_selector_reads_pinned_table`, `docs/SNAPSHOT_OPERATIONS.md` |
| Production-style Kubernetes example | External PostgreSQL/S3 profile with secret refs, pgwire TLS, metrics, resources | ⚠️ static example only | `just probe-static-check`, `deploy/kubernetes/` |
| Multi-modal asset footprint tables | WKB `footprint`/`geom` sidecar schemas | ✅ starter schema/discovery | `docs/MULTIMODAL_ASSETS.md`, `layoutbench_sf0`, `wire_spatial` footprint discovery |
| Real OSM parity | Geofabrik Monaco by default | ⚠️ opt-in | `just kind-osm-postgis-parity` covers OGR copy/read, MVT SQL bytes, and QGIS copy/read/render today |
| API/client profile surface | tokio-postgres local probe plus Kind Python probe for psycopg/SQLAlchemy/GeoPandas/pg_featureserv/BI/MVT-shaped SQL surfaces | ✅ maintained profile; named clients pending | `just api-client-local-smoke`, `just kind-api-client-probe`, `just kind-compatibility`, `docs/API_CLIENT_PROBES.md`; MVT profile asserts attribute tags |
| Real-data client matrix | OSM now; Overture/GeoParquet-style layers planned | ⏳ contract ready; broader execution required | `docs/REAL_DATA_CLIENT_MATRIX.md` |
| pg_featureserv, GeoPandas/SQLAlchemy, BI tools | stretch clients | ⏳ probe contract ready; not claimed | `docs/API_CLIENT_PROBES.md` |
| PostgreSQL logical replication / triggers / PL/pgSQL | PostgreSQL server features | ❌ non-goals | not part of QuackGIS architecture |

## Version policy

- The versions above are the versions QuackGIS actively probes by default.
- New client versions become supported only after a maintained trace/probe is
  added or an existing probe is intentionally retargeted.
- Opt-in probes can use newer images/binaries through Justfile environment
  overrides, but those runs are evidence for that environment, not a broad
  compatibility claim.
- PostgreSQL/S3 Alpha evidence currently uses in-cluster stand-ins. Production
  claims require external PostgreSQL/object-store failure-mode probes.

See [COMPATIBILITY.md](./COMPATIBILITY.md) for detailed limitations and
[OPERATIONS.md](./OPERATIONS.md) for how to run the probes.
