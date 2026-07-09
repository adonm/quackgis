# QuackGIS developer preview

This document defines the current coherent developer preview. It is the contract
for what a developer can run locally today without extra design context.

Project direction is broader than this local preview: QuackGIS is for
platform/application developers building high-throughput spatial services over a
shared DuckLake/Parquet data lake, including DuckDB-style columnar OLAP analysis
over filtered spatial records without embedding DuckDB. See
[PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md). The preview proves the local
shape before the Alpha promotion ladder (Kind+Linkerd, PostgreSQL catalog + S3,
parallel readers/writers, high-QPS gates, OLAP fanout probes, and operations
evidence).

## Preview goal

Run a single Rust binary that speaks enough PostGIS-compatible pgwire for common
GIS client loops while persisting data to DuckLake/Parquet and using SedonaDB for
spatial execution:

1. start QuackGIS with local SQLite catalog + local Parquet data;
2. create spatial tables over pgwire;
3. bulk load WKB geometry with PostgreSQL text `COPY FROM STDIN`;
4. query with PostGIS-style `ST_*` functions and client metadata shims;
5. repair fragmented appends with explicit compaction;
6. use the same tables from QGIS, GDAL/OGR, GeoServer, and Martin smoke paths.

The preview is intentionally single-node/local-storage first. PostgreSQL catalog
+ S3 object storage is not a side quest; it is the next alpha storage profile and
the scaled deployment target.

Preview priorities, in order:

1. PostGIS-compatible pgwire and catalog surface for existing GIS tools;
2. fast bulk ingest through PostgreSQL text `COPY FROM STDIN`;
3. QGIS/GDAL/OGR/GeoServer/Martin smoke compatibility;
4. WKB-first spatial layout and pruning for large analytical spatial queries;
5. columnar aggregate/calculation workloads over the pruned records;
6. explicit compaction to repair fragmented append layouts.

## One-command acceptance smoke

```sh
mise install
mise exec -- just preview-smoke
```

Expected tail:

```text
preview_table public.preview_points
preview_copy_rows 3
developer_preview_ok True
```

`preview-smoke` starts a temporary server, runs the
`developer_preview` pgwire example, and deletes the temporary catalog/data when
the recipe exits. The example exercises:

- `CREATE TABLE public.preview_points (...)`;
- PostgreSQL `COPY public.preview_points (...) FROM STDIN`;
- WKB geometry round-trip through `ST_AsText(ST_GeomFromWKB(...))`;
- `CALL quackgis_compact_table('public.preview_points')`;
- unchanged results after compaction.

## Manual local run

```sh
mise exec -- just server
```

In another shell:

```sh
psql -h 127.0.0.1 -p 5434 -U postgres -d quackgis
```

Useful SQL:

```sql
SELECT postgis_version();
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));

CREATE TABLE public.points (id INT, name TEXT, geom BINARY);
COPY public.points (id, name, geom) FROM STDIN;
1	origin	\\x010100000000000000000000000000000000000000
2	one	\\x0101000000000000000000f03f000000000000f03f
\.

SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) FROM public.points ORDER BY id;
CALL quackgis_compact_table('public.points');
```

For a Rust pgwire example against an already-running server:

```sh
cargo run -p quackgis-server --example developer_preview -- --host 127.0.0.1 --port 5434
```

## Current supported paths

| Area | Preview claim | Gate |
|---|---|---|
| Server | single Rust binary, local SQLite DuckLake catalog + local Parquet data | `just preview-smoke`, `just smoke-local-demo` |
| SQL writes | CTAS, `CREATE TABLE`, `INSERT`, `UPDATE`, `DELETE`, `RETURNING` for single DuckLake tables | `ducklake_persistence`, `wire_spatial` |
| Bulk ingest | PostgreSQL text `COPY FROM STDIN`, simple and extended pgwire, chunked `CopyData`, GDAL-style bytea/WKB escapes | `ducklake_*copy*` tests |
| Spatial query | WKB-first geometry with PostGIS-style `ST_*` aliases over SedonaDB | `wire_spatial`, `martin_compat` |
| Layout | hidden `_qg_*` bbox/bucket/sort columns, automatic projection, public metadata hiding | `layoutbench_sf0` |
| Pruning | safe bbox rewrite for supported single-table spatial predicates; exact SedonaDB predicate recheck | `layoutbench_sf0`, LayoutBench local runs |
| Compaction | explicit rewrite via `CALL quackgis_compact_table(...)`, with optional bucket-target arguments | `ducklake_compact_table_rewrites_without_changing_results`, `ducklake_compact_table_accepts_layout_bucket_scope`, LayoutBench compact mode |
| Client smoke | QGIS read/edit, GDAL/OGR load/read, GeoServer WFS/WMS/WFS-T, Martin tiles | Kind/manual compatibility probes; see `docs/COMPATIBILITY.md` |

## Performance levers proven so far

Current local LayoutBench `sf1` (`factor=100`) shows:

- COPY seed is about 18× faster than batched INSERT VALUES at this scale.
- Transaction/COPY grouping gives better layout than many autocommit INSERTs.
- Compaction repairs bad shuffled/autocommit layouts: aerial went from
  row-groups `22/18/4` to `22/1/21` and files/ranges `23/23/0` to `1/1/0`.
- Local row-group cap `QUACKGIS_DUCKLAKE_ROW_GROUP_ROWS=512` is the current best
  balance for sf1; set it to `0` to disable the override.

Reproduce the main local checks:

```sh
mise exec -- just layoutbench-sf0
mise exec -- just layoutbench-local-smoke

# Against an already-running server:
mise exec -- just layoutbench-local sf1 3 generated copy
mise exec -- just layoutbench-local sf1 3 shuffled insert true
```

## Known limitations

These are preview limitations, not bugs in the preview claim:

- Local preview storage is SQLite catalog + local Parquet. PostgreSQL catalog +
  S3-compatible storage is exercised by the Alpha Kind gates, but is not claimed
  by this local preview gate.
- `CALL quackgis_compact_table(...)` commits through one replacement snapshot for
  whole-table compaction. Optional bucket arguments use native bucket-local
  delete+append metadata when row-lineage planning succeeds, with the safe
  full-table replacement path retained as fallback.
- Transactions are single-table staged write transactions. DDL and multi-table
  write transactions fail closed; arbitrary in-transaction `SELECT` reads the
  committed catalog, not private staged rows.
- Autocommit DELETE and UPDATE use native DuckLake positional delete files through
  the vendored fork; UPDATE stages replacement rows and commits delete+append
  metadata in one snapshot. Explicit-transaction DML remains on the correct
  staged/full rewrite path.
- Spatial layout pruning only rewrites recognized safe single-table predicate
  shapes. Unsupported predicates still return correct exact SedonaDB results but
  may scan more.
- There is no PostgreSQL server, PL/pgSQL, triggers, LISTEN/NOTIFY, logical
  replication, `pg_dump` compatibility, or ctid.
- Auth is preview/dev oriented: default user `postgres`, database `quackgis`, no
  password. TLS is supported by flags, but production auth/RBAC hardening is a
  later milestone.

## Preview verification checklist

Before claiming a preview-ready commit:

```sh
mise exec -- just check-fast
mise exec -- just preview-smoke
mise exec -- just layoutbench-local-smoke
mise exec -- just layoutbench-sf0
git diff --check
```

Optional client/probe tier:

```sh
mise exec -- just kind-compatibility
mise exec -- just kind-compat-report
```
