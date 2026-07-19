# QuackGIS development stack

This directory contains the real-PostgreSQL QuackGIS proof stack. It is a
disposable development deployment, not yet a production release bundle.

## Proven path

```text
QGIS / GDAL / HTTP clients
          |
          +--> PostgreSQL 18 + PostGIS 3.6
          |        |
          |        +--> patched duckdb_fdw + local Quack client
          |                     |
          |                 127.0.0.1:9494
          |                     |
          |              dumbpipe / iroh 1.0
          |                     |
          |                 127.0.0.1:9494
          |                     |
          |              DuckDB 1.5.4 + Spatial + Quack
          |
          +--> Caddy --> pg_featureserv / Martin
```

The stack proves:

- native DuckDB WKB to `geometry(Point,4326)` conversion through the real
  PostGIS input function and attribute typmod;
- NULL geometry round-trip plus fail-closed malformed-WKB and wrong-family
  conversion;
- worker-side scalar and safe `LIMIT` pushdown;
- conservative worker-side numeric bbox candidates for PostGIS `&&` and
  `ST_Intersects`, with the original PostGIS predicate retained locally;
- the exact query shapes used by QGIS, GDAL/OGR, Martin, and
  `pg_featureserv`;
- loopback-only Quack endpoints carried by pinned `dumbpipe` 0.39.0 over
  iroh, including fail-closed behavior when the client tunnel is stopped;
- separate read-only PostgreSQL roles for direct clients, features, and tiles;
- Caddy routes for OGC API Features, dynamic MVT/TileJSON, and an immutable
  PMTiles-backed revision fixture; and
- build-time installation and SHA-256 verification of DuckDB 1.5.4, Quack,
  HTTPFS, and Spatial artifacts. Runtime uses `LOAD`, not `INSTALL`.

## Run

```sh
just quackgis-up
just quackgis-smoke
just quackgis-plan
just quackgis-http-smoke
just quackgis-transport-smoke
just quackgis-client-smoke
just quackgis-down
```

`quackgis-client-smoke` uses the digest-pinned QGIS 3.44.11 image already used
by the repository and also exercises its GDAL 3.10.3 `ogrinfo` client. The
other checks need `psql`, `curl`, and Python 3 on the host.

Development endpoints:

- PostgreSQL: `127.0.0.1:55432`, database `quackgis`;
- Caddy: `http://127.0.0.1:8080`;
- OGC API Features: `http://127.0.0.1:8080/features/`;
- dynamic TileJSON: `http://127.0.0.1:8080/tiles/features`; and
- immutable PMTiles fixture TileJSON:
  `http://127.0.0.1:8080/tiles/revision-f3f65093582c`.

Quack, Martin, and `pg_featureserv` have no host-published ports.

## Tracked patches

`postgres/patches/quackgis.patch` applies to `alitrack/duckdb_fdw` commit
`9354241029df691b695f15428082b7c5cd81e2c7`. It contains:

- PostgreSQL 18 planner and EXPLAIN API compatibility;
- the development-only plain-HTTP Quack option used on loopback;
- custom-type typmod preservation;
- WKB-hex projection for PostGIS geometry;
- narrow PostGIS `&&`/`ST_Intersects` bbox translation;
- safe simple-scan `LIMIT` pushdown;
- complete virtual-slot NULL initialization; and
- runtime `LOAD` of the preinstalled Quack artifact.

`pg_featureserv/patches/quackgis.patch` applies to release 1.3.1 commit
`2f1bd809f2c745d6d332495ee336f604f3ad9063`. It loads bounded, published
extent metadata for the collections listing instead of returning zero extents
or issuing an unbounded `ST_Extent` fallback. For an explicitly allowlisted
view, an integer `id` column is treated as the stable GeoJSON feature identity;
the publisher is responsible for its uniqueness.

Patch applicability and both upstream `pg_featureserv` test groups run during
image builds.

## Build and runtime network behavior

A first build needs outbound access for pinned source repositories, base
images, Go/Cargo modules, and DuckDB extension artifacts. Extension bytes are
then checked against tracked SHA-256 values and embedded in the worker and
PostgreSQL images. A container restart does not download extensions.

The current extension checksums are for `linux_amd64`; the Dockerfiles fail
closed on another architecture. ARM64 needs separately pinned and tested
artifacts.

`dumbpipe` currently selects iroh's public N0 relay preset while preferring a
direct path. The stack works through the exchanged endpoint ticket and keeps
both TCP application sockets on loopback, but production still needs an
operator-selected relay policy and a tunnel that allowlists the client endpoint
identity.

## Development-only credentials

The checked-in PostgreSQL passwords, Quack token, and iroh keys are disposable
local defaults. They are intentionally visible so this proof is reproducible.
Do not reuse them, expose this Compose file to an untrusted network, or treat
it as production secret management.

## PMTiles fixture

`martin/fixtures/test_fixture_1.pmtiles` is the 468-byte upstream go-pmtiles
test fixture from commit `3fd9b032fa04213473c933d22a447d36b3a8d51b`, with SHA-256
`f3f65093582c81625cdfab11b3d1a27c2fb6aadcf8bd9802b2ad694d4d7cdca5`.
Its BSD-3-Clause notice is retained beside the fixture. It proves immutable
Martin routing only; a release needs a real revision publication pipeline.
