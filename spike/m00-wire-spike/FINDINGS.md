# M0 wire spike — findings

**Probe re-run: 2026-07-05 (round 2)** after switching to upstream
datafusion-postgres master + a SedonaDB fork-bump to DF 53. **Original probe
(2026-07-05 round 1) used `=0.15.0` pinned — see commit history.** This file
documents the current state.

## Headline

**Riding both upstream heads works.** Switched off the v0.15 pin and onto:

- `datafusion-postgres` master (DataFusion 53 / Arrow 58) — consumed directly
  from `datafusion-contrib/datafusion-postgres` (no fork yet — we don't need
  local patches in this milestone).
- `sedona` from `adonm/sedona-db@quackgis/df53` — SedonaDB main is still on
  DataFusion 52; this fork-bump aligns it with DF 53 so the two upstreams
  compose on a single `SessionContext`. Fork commit `f274c942`, 8 files
  changed (all mechanical API adjustments — see the fork's commit message).

All gates green again:

```
$ psql -h 127.0.0.1 -p 5434 -U postgres -c "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))"
 POINT(1 2)
$ SELECT ST_Area(...)            -> 16.0
$ SELECT ST_Intersects(...)      -> t
$ SELECT 1+1                     -> 2
```

## New wins from moving to master

- **G3 cursors mostly closed.** `DECLARE c CURSOR FOR ...` / `FETCH FORWARD n` /
  `CLOSE c` all work natively — `CursorStatementHook` is in master's default
  hook list (it was added after v0.15). The previous round-1 fork-backport
  plan for cursors is **no longer needed**.
- **G3 BINARY-format gap narrowed to a perf issue.** `DECLARE BINARY CURSOR`
  returns the WKB as hex-text bytea (`\x010100...`) rather than true binary
  protocol bytes. The bytes are valid WKB and decode correctly — QGIS reads
  bytea fine in either form, it just loses the ~2× bandwidth saving that
  BINARY cursors exist to provide. Not a correctness gap, just an efficiency
  one. Fork scope: extend `CursorStatementHook` to honor the BINARY keyword
  by switching the encoding path. Small.
- **G11 (DF version alignment) resolved by fork-bump**, not by pinning back.
  Riding both upstream heads as requested.

## SedonaDB DF 53 bump — what it took

8 files, all mechanical, no logic changes (`adonm/sedona-db@f274c942`):

- `ExecutionPlan::properties()` returns `&Arc<PlanProperties>` instead of
  `&PlanProperties` (5 sites: probe_shuffle_exec, exec, random_geometry_provider,
  record_batch_reader_provider, plus field-type changes)
- `Join::try_new` gained `null_aware: bool` (optimizer.rs)
- `PartitionedFile` gained `ordering: Option<LexOrdering>` (datasource/provider.rs)
- `object_store 0.12 → 0.13` (workspace dep — was the source of the dual
  `object_store::Path` types in the graph)
- `Option<&Vec<usize>> → Option<&[usize]>` coercions (spatial-join exec, 2 sites)

Upstream PR candidate once SedonaDB cuts on DF 53.

## Open gap ledger status (post-round-2)

| # | Status |
|---|---|
| G1 | Untested in spike — arrow-pg geometry encoding fork work still ahead (M2) |
| G2 | pg_catalog depth work still ahead (M3) |
| G3 | **Cursors work for the simple-query path.** Two sub-gaps confirmed by the M0 wire test (commit b5cbeff): (a) BINARY keyword accepted but encoding stays hex-text bytea (perf gap, not correctness); (b) FETCH via extended protocol fails — `CursorStatementHook` stores a portal with no schema and FETCH emits DataRows without a matching RowDescription. Upstream `CursorStatementHook` needs a small fork for both. |
| G4 | Extended protocol works for SELECT/PREPARE/EXECUTE; JDBC `setFetchSize` portal suspension still needs a pgjdbc probe (likely blocked on G3(b) being fixed first) |
| G5–G8 | DuckLake work, unchanged from prior assessment (M1+) |
| G9 | SedonaDB consumable as a git dep — confirmed (now from our DF 53 fork) |
| G10 | BEGIN/COMMIT/ROLLBACK accepted as no-ops via `TransactionStatementHook` |
| G11 | **Resolved by fork-bump** (`adonm/sedona-db@quackgis/df53`) |
| G12 | Runtime libgeos (`libgeos_c.so.1`) still required |

## Build artifacts

- 8m48s clean build of the spike against upstream master + fork head.
- 7m10s for the in-fork `cargo build -p sedona` (verifies the bump).
- Subsequent edits incremental.

## Recommendation

M0 (wire) is unblocked. Next is the M0 retirement of v0.1 + standing up the
forks as proper `[patch.crates-io]` consumers in a real `quackgis-server`
crate. SedonaDB DF 53 fork-bump is upstream-PR candidate — file it once the
release cycle would naturally move SedonaDB to DF 53.
