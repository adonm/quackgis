# DuckLake alignment ledger

QuackGIS treats DuckLake as the durable storage contract. Fork-backed behavior is
allowed only when it has an explicit migration path back to official DuckLake
semantics or a documented reason to remain QuackGIS-local.

This ledger is the release-facing checklist for every storage behavior that could
affect catalog/data compatibility.

## Alignment rules

1. Prefer official DuckLake behavior when it is stable enough for QuackGIS gates.
2. Keep QuackGIS fork patches behind small, named capabilities with tests that
   fail without the patch.
3. Do not expose private object/catalog details as user-facing APIs.
4. Validate both SQLite/local and PostgreSQL/S3 profiles before widening a storage
   claim.
5. Before a release-to-release upgrade claim, copy the catalog + object prefix and
   prove table discovery, counts, bboxes, representative spatial reads,
   `geometry_columns`, and compaction on the copied environment.

## Current behavior map

| QuackGIS behavior | Current implementation | Upstream DuckLake direction | Interop/evidence gate | Migration trigger |
|---|---|---|---|---|
| SQLite/local profile | DuckLake catalog in SQLite plus local Parquet files | Official local catalog/data path | `just check-fast`, `just preview-smoke`, persistence tests | Keep as deterministic preview path; no migration expected |
| PostgreSQL/S3 profile | `datafusion-ducklake` PostgreSQL catalog plus S3-compatible object data | Official SQL catalog + object-store profile, fewer PostgreSQL roundtrips | `just kind-alpha-smoke`, `docs/ALPHA_EXTERNAL_SERVICES.md` for real services | Replace emulator-only claims with external-service evidence |
| Geometry storage | Spec `GEOMETRY`/WKB columns plus EWKB at pgwire boundaries | DuckLake GEOMETRY type and future UDT/type metadata | `wire_spatial`, `postgis_regress`, client probes | Adopt richer upstream type metadata only when it preserves WKB client compatibility |
| Hidden spatial layout columns | QuackGIS-owned `_qg_*` bbox/bucket/sort columns maintained on writes | DuckLake partitioning/statistics/Bloom/metadata pruning improvements | `layoutbench_sf0`, QPS/OLAP scan budgets, compaction scan tests | Prefer upstream pruning/statistics when it can replace hidden columns without losing exact recheck guarantees |
| Autocommit native `DELETE` | Fork-backed positional delete files committed under one snapshot | Multi-deletion-vector/Puffin evolution | `ducklake_delete_uses_atomic_native_delete_files_across_data_files`, external native-delete probe | Move to upstream deletion-vector primitives when they can commit all affected files atomically |
| Autocommit native `UPDATE` | Fork-backed delete files plus pending replacement data file under one snapshot | Official update/delete mutation APIs over deletion vectors and appended data | `ducklake_update_uses_atomic_native_delete_and_append` | Migrate when upstream can preserve one visible snapshot boundary and `RETURNING` semantics |
| Bucket-local compaction | Fork-backed delete+append mutation when row-lineage planning succeeds; full replacement fallback | Official compaction/optimization APIs and deletion-vector maintenance | `ducklake_compact_table_accepts_layout_bucket_scope`, scan-byte/row-group compaction test | Prefer official compaction primitives once bucket-local replacements are atomic and reference-reader-safe |
| Full-table compaction | QuackGIS `CALL quackgis_compact_table(...)` replacement snapshot | DuckLake optimize/compaction procedures | `ducklake_compact_table_rewrites_without_changing_results` | Replace or alias to upstream procedure when available without changing client contract |
| Metadata inspection | Safe pgwire UDTFs: `ducklake_snapshots()`, `ducklake_table_info()`, `ducklake_list_files()` | DuckLake metadata tables/functions | `ducklake_metadata_table_functions_roundtrip_through_wire` | Track upstream metadata schemas; keep QuackGIS wrappers stable for clients |
| CDC row functions | Disabled after pgwire projection panic | Official change-data/log table functions or snapshot diffs | Disabled until safe projection test exists | Re-enable only after pgwire/Arrow projection is safe and row semantics are documented |
| SQL time travel | Not implemented; plan documented in `docs/SNAPSHOT_OPERATIONS.md` | Protected snapshots and `AS OF`/time-travel reads | future SQL `AS OF` tests | Implement against upstream snapshot APIs where possible |
| Backup/restore | Local SQLite/filesystem oracle plus external/snapshot runbooks | Protected snapshots / snapshot retention | `ducklake_local_backup_restore_copy_roundtrip`, `docs/ALPHA_EXTERNAL_SERVICES.md`, `docs/SNAPSHOT_OPERATIONS.md` | Use protected snapshots when upstream exposes stable retention semantics |
| Branch/merge/staged imports | Not implemented | DuckLake branch/merge roadmap | future staged-import/edit-review probes | Prefer upstream branch/merge over QuackGIS-specific staging catalogs |
| Materialized summaries | Not implemented beyond ordinary tables/queries | DuckLake materialized views / incremental maintenance | future tile/coverage/asset summary probes | Use upstream materialized views before bespoke refresh machinery |
| Asset metadata | Ordinary sidecar tables with WKB `footprint`, URIs, scalar metadata | VARIANT/UDT/fixed-size-array support | `docs/MULTIMODAL_ASSETS.md`, footprint discovery tests | Move semi-structured metadata/calibration arrays to stable upstream types, not custom binary islands |
| Catalog/read performance | Read refresh interval plus QPS/OLAP scan budgets | Bloom filters, metadata scan improvements, fewer catalog roundtrips | `kind-qps-smoke`, `kind-olap-smoke`, `metrics-dashboard.md` | Prefer upstream metadata scan/roundtrip optimizations over ad hoc caching |

## Reference-reader policy

QuackGIS should not require external readers to understand QuackGIS-only private
metadata for ordinary table scans. Before claiming reference-reader compatibility
for a release, copy a representative catalog + object prefix and verify:

- table list and schemas;
- row counts and representative sample rows;
- delete-file semantics after native `DELETE`/`UPDATE`;
- compacted table reads;
- geometry columns as WKB/GEOMETRY data;
- failure behavior for QuackGIS-only metadata functions.

If a behavior is correct for QuackGIS but not yet safe for reference readers,
release notes must say so and name the upstream feature that will close the gap.

## Migration decision record template

Use this template in PRs that change storage compatibility:

```text
Behavior:
Current QuackGIS implementation:
Upstream DuckLake feature or issue:
Catalog/data compatibility risk:
Gates run:
Reference-reader result:
Migration trigger / rollback plan:
```

## Current release stance

The maintained release stance is conservative: DuckLake is the durable contract,
QuackGIS may fork to preserve one-snapshot spatial write semantics, and official
DuckLake primitives should replace fork-backed behavior when they satisfy the same
correctness and client-compatibility gates.
