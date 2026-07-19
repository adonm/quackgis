# QuackGIS project history

This is selective context for retired architecture. Current truth lives in
`PROJECT_DIRECTION.md`, `ARCHITECTURE.md`, `ROADMAP.md`, `ROADMAP_STATUS.md`,
and `NATIVE_BUNDLE.md`.

## Evolution

### DuckDB spatial extension prototype

The first prototype explored a PostGIS-like WKB surface implemented as a DuckDB
extension with GeoRust algorithms. It established useful ideas—vectorized
execution, WKB interchange, semantic fixtures, and spatial benchmarks—but did not
solve pgwire clients, authorization, shared storage, or operations.

### Sedona/DataFusion and forked DuckLake

The next architecture used a Rust pgwire service over DataFusion, SedonaDB, and a
forked `datafusion-ducklake`. It developed important contracts for parsed policy,
COPY, client traces, WKB identity, snapshot publication, exact spatial recheck,
and failure evidence. It also accumulated planner, catalog, storage-writer, and
compatibility ownership that duplicated the engine/storage ecosystem.

### DuckDB/official DuckLake cutover

The project selected DuckDB as the sole planner/spatial executor and official
DuckLake as the sole new storage writer. The Rust edge retained protocol,
authorization, compatibility, Arrow encoding, and lifecycle policy. DataFusion,
Sedona SQL execution, forked storage/catalog code, and their vendor trees were
removed in July 2026.

The current architecture deliberately does not retain an auxiliary engine. Missing
spatial behavior follows the native → SQL macro/rewrite → Rust edge → vectorized
DuckDB extension ladder.

## Retained lessons

- WKB/EWKB is a useful stable wire/interchange format.
- Exact spatial predicates remain authoritative; candidate filters may only
  over-select.
- PostgreSQL compatibility is trace-driven, not a request to emulate PostgreSQL.
- Storage publication, protocol state, and authorization are separate boundaries.
- Bulk COPY and Arrow streaming matter more than row-wise compatibility paths.
- Claims require executable evidence from the current runtime.
- Open storage and independent DuckDB reopen are stronger than internal catalog
  cleverness.
- Native artifacts and extensions are product dependencies requiring provenance,
  upgrade, and recovery gates.

## Archaeology anchors

Git history remains the full archive. Useful starting points:

| Anchor | Era |
|---|---|
| `8788859` | original DuckDB WKB/GeoRust extension prototype |
| `e602e49` | DuckLake + Sedona bridge exploration |
| `bc2b835` | PostgreSQL facade prototype |
| `1acd774` | Rust pgwire pivot |
| `37db454` | retirement of first-party DataFusion engine code |
| `81328a3` | removal of DataFusion vendor trees |
| `6757101` | DuckDB-owned PostGIS compatibility edge |

Use `git show <anchor>` when a historical semantic fixture or implementation idea
is needed. Do not restore old code or claims without revalidating them against the
current direction and registered DuckDB gates.
