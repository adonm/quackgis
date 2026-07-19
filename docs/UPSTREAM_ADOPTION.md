# Upstream adoption and deletion review

QuackGIS checks current upstream DuckDB, DuckLake, and Spatial before adding or
retaining native code. The goal is to adopt released upstream behavior and delete
local machinery, not to preserve patches, compatibility macros, metadata adapters,
or side indexes as project assets.

The machine-readable review is [`native/upstream-review.json`](../native/upstream-review.json).
`just native-bundle-check` validates that every selected source and tracked patch
has a review. The opt-in network command compares the recorded review with current
upstream refs:

```sh
mise exec -- just native-upstream-check
```

This command is deliberately not a network-dependent CI gate. If a release or
reviewed branch moves, it fails and requires a human to inspect new capabilities,
update adopt/retain/delete decisions, and freeze new exact commits. It never
silently changes `native/bundle.json`.

## What “latest” means

Two upstream states are reviewed, with different authority:

1. **Latest supported release:** first choice for a runtime bundle. A release can
   be selected only after the complete build, compatibility, reopen, recovery,
   package, client, and performance matrix passes.
2. **Latest compatible branch and `main`:** evidence about code that may soon
   remove QuackGIS behavior. Floating tips, nightlies, roadmap entries, and future
   releases are never runtime selectors. They trigger probes and deletion plans,
   not production dependencies.

A newer branch is not “compatible” merely because its branch name matches. Its
embedded/declared DuckDB core commit must equal the candidate core or be rebuilt
and qualified as a new atomic bundle. QuackGIS does not mix a newer extension
source with an older released core to claim upstream adoption.

## Required workflow before local native work

Before adding a patch, QuackGIS extension function, catalog adapter, spatial
rewrite, or maintained side structure:

1. run `just native-upstream-check` and inspect the latest published DuckDB
   release plus DuckDB-versioned DuckLake/Spatial branches and `main`;
2. search the exact sources for the required public function, type, lifecycle
   hook, planner behavior, or metadata surface;
3. run the QuackGIS oracle unchanged against the unmodified release-matched
   candidate (still an open N0 acceptance step for the current bundle);
4. adopt upstream behavior and delete overlapping local code when the oracle
   passes;
5. if upstream is close but incomplete, prefer an upstream contribution and keep
   only the smallest temporary patch with an explicit deletion gate; and
6. add local native behavior only when the machine-readable review records the
   missing upstream capability, exact commits searched, requirement, tests,
   owner, and deletion plan.

Local behavior is not kept merely because it already works. Before a candidate is
accepted, its update workflow must run the same oracle against unmodified and
patched source and record patch deletion review. The current N0 preparer builds the
patched candidate; the corresponding pristine differential run remains an open
acceptance gate, not completed evidence. Metadata-only or unenforced upstream
declarations are also not overclaimed: adoption still requires the product's
correctness and lifecycle gates.

## Review on 2026-07-19

| Component | Current upstream evidence | Decision |
|---|---|---|
| DuckDB | [v1.5.4](https://github.com/duckdb/duckdb/releases/tag/v1.5.4) at `08e34c447bae34eaee3723cac61f2878b6bdf787` is the latest published non-LTS release | selected as the current bundle core; newer branch commits remain candidate evidence |
| DuckLake | `v1.5-variegata` is `84ef2d14a0161f6f6197d6c8d2b4dbc45bf40375`, the selected commit; current `main` is also reviewed | use official writer/lifecycle behavior; retain the column-identity patch only because neither reviewed tree exposes an equivalent public function |
| Spatial | selected `28db190f7184bcf61eb01d291e0cba79849bddb6` matches released DuckDB 1.5.4; the newer 1.5 tip targets an unreleased DuckDB commit | retain the release-matched source now; qualify newer upstream R-tree/spatial-join work only in a matching candidate bundle |

The review produced concrete deletion/adoption decisions:

- **CRS:** selected upstream Spatial already contains CRS-parameterized
  `GEOMETRY`, `ST_CRS`/`ST_SetCRS` semantics, CRS-aware transforms, propagation,
  and mismatch tests. S0 must adopt and qualify those semantics through DuckLake,
  pgwire, OGR, QGIS, and migration. It must not create a parallel Spatial fork or
  QuackGIS CRS engine.
- **Column identity:** selected DuckLake and current `main` expose
  `ducklake_table_info` but no public `ducklake_column_info` equivalent. The
  read-only patch remains a temporary upstream gap with complete lifecycle tests
  and a mandatory deletion check on every update.
- **Spatial pruning:** DuckDB 1.5.4 native geometry statistics already participate
  in M4 evidence. Newer unreleased Spatial commits add R-tree scan/filtering,
  parallel fetch, CRS, and spatial-join improvements. Each release-matched bundle
  must rerun the exact write/client/plan/scan matrix and delete maintained WKB bbox
  machinery when upstream meets those gates.
- **Storage lifecycle:** official DuckLake remains the only writer, snapshot, and
  compaction implementation. QuackGIS does not duplicate it with a private writer
  or metadata format.

The exact observed refs and capability dispositions live in the machine-readable
review so a moved upstream cannot be mistaken for already-reviewed behavior.
