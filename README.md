# QuackGIS

QuackGIS is a small, read-oriented geospatial edge built from existing database
and web protocols instead of a custom PostgreSQL implementation.

```text
QGIS · GeoServer · GDAL/OGR · applications
                    │ PostgreSQL/PostGIS
                    ▼
          PostgreSQL 18 + PostGIS 3.6
                    │ duckdb_fdw + Quack
                    ▼
            local iroh tunnel/proxy
                    │
                    ▼
          DuckDB Spatial / DuckLake worker

HTTP clients ──► Caddy ──┬──► Martin (MVT, TileJSON, PMTiles)
                         └──► pg_featureserv (OGC API Features)
```

## Status

**Architecture reset / working proof of concept.** The previous owned Rust pgwire and
PostGIS-emulation implementation is preserved in Git history at `17a7710`. It is
not the implementation direction of this branch.

The critical path now passes in the development stack: `duckdb_fdw` reaches a
DuckDB worker through a loopback iroh tunnel, transports native WKB as
`geometry(Point,4326)`, and pushes QGIS, GDAL/OGR, Martin, and
`pg_featureserv` bbox candidates to the worker while retaining exact PostGIS
rechecks. Real QGIS 3.44 and GDAL/OGR clients, OGC API Features, dynamic MVT,
TileJSON, and an immutable PMTiles fixture are exercised end to end.

## First release

The first release will provide:

- a real PostgreSQL/PostGIS endpoint for read-only QGIS, external GeoServer, and
  GDAL/OGR clients;
- remote DuckDB/DuckLake reads through `duckdb_fdw`, Quack, and an iroh-local
  transport endpoint;
- native PostGIS geometry columns with fixed, explicit CRS metadata;
- worker-side attribute, projection, limit, and viewport bbox filtering;
- Caddy as the only public HTTP entry point;
- Martin endpoints for TileJSON, MVT, and immutable PMTiles;
- a required `pg_featureserv` OGC API Features endpoint that can be replicated
  behind an HTTP load balancer; and
- immutable, revision-addressed cache URLs.

GeoServer and MapServer are not bundled. GeoServer remains a supported external
PostgreSQL client. Writes, multi-worker scheduling, broad PostGIS function
translation, and a general-purpose control plane are post-first-release work.

## Start the current proof of concept

```sh
just quackgis-up
just quackgis-smoke
just quackgis-plan
just quackgis-http-smoke
just quackgis-transport-smoke
just quackgis-client-smoke
just quackgis-down
```

The stack requires Docker Compose or Podman with a Docker Compose provider.
The first image build needs outbound access. DuckDB and extension artifacts are
version/checksum-pinned into the built images; runtime containers do not install
extensions. Current extension checksums are `linux_amd64` only.

Read:

- [docs/PROJECT_DIRECTION.md](docs/PROJECT_DIRECTION.md) — goals and non-goals.
- [ARCHITECTURE.md](ARCHITECTURE.md) — component and trust boundaries.
- [ROADMAP.md](ROADMAP.md) — ordered first-release gates.
- [deploy/quackgis/README.md](deploy/quackgis/README.md) — proof-of-concept commands and known gaps.
