# Fork divergence ledger

Tracks every fork QuackGIS consumes, the upstream it tracks, and what differs
from upstream HEAD. Policy: ride upstream heads; fork when a gap blocks us;
every fork divergence is an upstream PR candidate after enough local soak.

Status: 🟢 in sync · 🟡 local patches, upstreamable · 🔴 blocked.

## Active forks

### `adonm/sedona-db` — 🟡 upstreamable

- **Upstream:** `apache/sedona-db` (`main`, currently DF 52.5 / Arrow 57).
- **Consumed via:** root `Cargo.toml`, branch `quackgis/df54`.
- **Head:** `2f00283`.
- **Purpose:** target DuckLake 1.0+ via `datafusion-ducklake` main, which is on
  DataFusion 54. SedonaDB upstream lags (DF 52.5), so the fork aligns SedonaDB
  with the QuackGIS stack (DF 54 / Arrow 58 / object_store 0.13).
- **Diff:** mechanical API adaptation only; no kernel/semantic changes.
  - DF 52→53: `PlanProperties` now `Arc`, `Join::try_new` `null_aware` arg,
    `PartitionedFile.ordering`, `object_store 0.13`, `&Vec<usize> -> &[usize]`.
  - DF 53→54: removed `as_any` from many DF traits in favour of inherent
    `downcast_ref`; keep Arrow `Array::as_any()` and DF `ExtensionOptions` /
    `UserDefinedLogicalNode::as_any()` where still required. `MetricType` casing,
    `partition_statistics -> Arc<Statistics>`, `PartitionedFile.table_reference`,
    `MemoryPool::{name, Display}`, generic accumulator `'static` bounds.
- **Upstream plan:** PR after local M1/M2 soak. The bump is large enough that
  submitting after quality coverage is preferable.

### `adonm/datafusion-postgres` — 🟡 upstreamable

- **Upstream:** `datafusion-contrib/datafusion-postgres` (`master`, currently
  DF 53 / Arrow 58).
- **Consumed via:** root `Cargo.toml`, branch `quackgis/fixes`.
- **Head:** `2c2e5d9` (pushed to `adonm/datafusion-postgres@quackgis/fixes`).
- **Purpose:** track QuackGIS stack (DF 54) and carry correctness + client-
  compatibility patches found by M2 probes (psql, tokio-postgres, Martin).
- **Patches:**
  1. `2c43dc6` — fix `PgCatalogContextProvider for Arc<T>` infinite recursion.
     `self.roles()` / `self.role()` resolved to the Arc impl itself, causing
     stack overflow on `pg_catalog.pg_roles`. Fix: `(**self).roles()` / `role()`.
  2. `98b3865` — honour `DECLARE ... BINARY CURSOR` by using
     `Format::UnifiedBinary` and propagating the result format to the portal.
  3. `2b17034` — mechanical DF 53→54 bump.
  4. `912823e` — `::geometry`/`::geography` cast preprocessing to `::bytea`.
  5. `81a2b68` — `&&` (PGOverlap) operator rewrite to `st_overlaps_bbox`.
  6. `445f7cb` — `::jsonb` cast rewrite to `::varchar` for Martin discovery.
  7. `7c73826` — shortcut Martin table-discovery query to `geometry_columns`
     projection (avoids full pg_index/pg_opclass/jsonb machinery).
  8. `a42a948` — return NULL `relkind` in Martin discovery shortcut.
  9. `f027b2f` — map Arrow `Int8` to PostgreSQL internal `"char"` (OID 18)
     so `pg_class.relkind` decodes correctly via tokio-postgres.
  10. `b548ef3` — rewrite `PGOverlap` inside derived-table subqueries.
  11. `98a65d4` — rewrite `ST_AsMVT(tile, ...)` record form to `ST_AsMVT(tile.geom)`.
  12. `b81c65c` — encode Martin discovery `properties` (Utf8) as JSONB.
  13. `67399c5` — shortcut Martin function-discovery query to empty result
      (QuackGIS has no tile-generating SQL functions).
  14. `25eab17` — rewrite Martin's `ST_TileEnvelope(..., margin => 0.015625)`
      named argument to positional, matching QuackGIS' margin overload.
  15. `93f8273` — rewrite PostGIS fixture DDL before parsing:
      `CREATE EXTENSION ...` and PL/pgSQL `DO $$ ... $$` blocks become no-ops;
      `serial`/`bigserial` become `int`/`bigint`; `GEOMETRY(...)` and
      `GEOGRAPHY(...)` column types become `BYTEA` for DataFusion DDL;
      `CREATE INDEX`, `CLUSTER`, and `COMMENT ON` become no-ops; and
      `CREATE MATERIALIZED VIEW` is lowered to `CREATE VIEW`.
  16. `8958716` — sanitize pathological PostgreSQL quoted identifiers into
      deterministic safe quoted names before sqlparser sees fixture DDL. This
      closes the upstream Martin `SpacesAndQuotes.sql` fixture without changing
      the fixture input.
  17. `2c35282` — advertise PostGIS-compatible `geometry`/`geography` type OIDs
      for binary WKB columns whose name matches the spatial naming convention
      (`geom`, `geometry`, `the_geom`, `wkb_geometry`, `wkb_geom`, `shape`,
      `way` / `geog`, `geography`, `the_geog`), in both RowDescription
      (`arrow-pg::datatypes::field_into_pg_type`) and `pg_attribute.atttypid`
      (`datafusion_field_to_pg_type`). Wire encoding is unchanged (raw WKB /
      hex-EWKB), so existing bytea clients are unaffected; QGIS/GeoServer now
      see a distinct spatial type OID instead of `bytea`. Sentinel OIDs
      `GEOMETRY_OID=90001` / `GEOGRAPHY_OID=90002` are used because the PostGIS
      type OIDs are dynamic per-install.
  18. `bc75e99` — handle DataFusion 54's casted oid-string literal shape in
      the oid coercion analyzer rule (`CAST('public' AS Int32)` /
      `TRY_CAST(...)`) and update tests to the DF54 `Cast`/`TryCast` API.
  19. `2c2e5d9` — make custom spatial type OIDs resolvable by clients such as
      tokio-postgres: oid-like catalog fields now advertise PostgreSQL `oid`,
      internal type-info queries bind `$1` as `oid`, `oid` parameters deserialize
      back to DataFusion integer values, and geometry/geography WKB payloads are
      encoded through bytea-compatible serializers while retaining the advertised
      geometry/geography OIDs.
- **Remaining fork target:** G3(b), extended-protocol `FETCH` RowDescription
  mismatch (`DataRow field count does not match`). Not blocking QGIS/libpq.
- **Upstream plan:** split into small PRs after local soak: Arc recursion fix
  first, binary cursor second, DF54 bump if upstream has not already moved.
  Martin-specific rewrites (7-14) are QuackGIS-specific and stay in-fork.

### `adonm/datafusion-ducklake` — 🟢 no local patches yet

- **Upstream:** `datafusion-contrib/datafusion-ducklake` (`main`, currently DF
  54 / Arrow 58). This is the Rust-native path closest to official DuckLake
  v1.0+.
- **Consumed via:** root `Cargo.toml`, branch `main`.
- **Head:** follows upstream fork main (no QuackGIS patches yet).
- **Known upcoming fork targets:** G5 UPDATE/DELETE write path and G7
  file/partition pruning. M1 routes CTAS, bare CREATE, INSERT SELECT/VALUES, UPDATE, and DELETE through a QuackGIS hook into the writer API using full-table rewrite semantics where needed. Generic filter pushdown path is covered by tests; spatial-layout pruning remains M5.

## Rebase hygiene

- Default branches track upstream. QuackGIS patches live on `quackgis/*`.
- Rebase `quackgis/*` at milestone boundaries; avoid upstream PRs until local
  tests prove the patch across QuackGIS workflows.
