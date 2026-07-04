# Fork divergence ledger

Tracks every fork QuackGIS consumes, the upstream it tracks, and what (if
anything) differs from upstream HEAD. Per the policy in
[CONTRIBUTING.md](./CONTRIBUTING.md) and [ROADMAP.md](./ROADMAP.md): we ride
upstream heads; we fork only when an upstream gap blocks us; every fork
divergence is a candidate upstream PR.

Status legend: 🟢 in sync with upstream HEAD · 🟡 local patches, upstreamable ·
🔴 local patches, blocked on a design decision.

## Active forks

### `adonm/sedona-db` — 🟡 upstreamable

- **Upstream:** `apache/sedona-db` (`main`).
- **Consumed via:** `[workspace.dependencies]` git dep on branch
  `quackgis/df53` from the root `Cargo.toml`.
- **Default branch (`main`):** in sync with upstream.
- **Working branch (`quackgis/df53`):**
  - Base: `apache/sedona-db@1eca227f` (upstream `main` at fork time).
  - Head: `f274c942`.
  - **Purpose:** bump DataFusion 52.5 → 53, Arrow 57 → 58, `object_store`
    0.12 → 0.13 so SedonaDB composes with `datafusion-postgres` master
    (which moved to DF 53 / Arrow 58). Without this, two incompatible
    `SessionContext` types end up in the dep graph (gap ledger **G11**).
  - **Diff:** 8 files, all mechanical. No logic changes.
    - `Cargo.toml` workspace dep version bumps (3 entries)
    - `ExecutionPlan::properties()` returns `&Arc<PlanProperties>` in DF 53,
      not `&PlanProperties` — 5 sites (probe_shuffle_exec, spatial-join exec,
      random_geometry_provider, record_batch_reader_provider + their field
      types)
    - `Join::try_new` gained `null_aware: bool` arg
      (`rust/sedona-query-planner/src/optimizer.rs`)
    - `PartitionedFile` gained `ordering: Option<LexOrdering>` field
      (`rust/sedona-datasource/src/provider.rs`)
    - `Option<&Vec<usize>> → Option<&[usize]>` coercions
      (`rust/sedona-spatial-join/src/exec.rs`, 2 sites)
    - Full commit message has details.
  - **Upstream plan:** file as a PR against `apache/sedona-db` when their
    release cycle moves to DF 53 (no current indication of when; SedonaDB
    is on a slower release cadence than DataFusion). Until then, rebase
    `quackgis/df53` onto new upstream `main` commits as they land.
  - **Rebase expectations:** mechanical (API adjustments only); typical
    upstream change touches kernel code, not the `properties()`/`Join`/
    `PartitionedFile` boundary. Low drift risk.

### `adonm/datafusion-postgres` — 🟡 upstreamable, one patch

- **Upstream:** `datafusion-contrib/datafusion-postgres` (`master`).
- **Consumed via:** `[workspace.dependencies]` git dep on branch
  `quackgis/fixes` from the root `Cargo.toml`.
- **Default branch (`master`):** in sync with upstream.
- **Working branch (`quackgis/fixes`):**
  - Base: `datafusion-contrib/datafusion-postgres@293d64a8` (upstream `master`
    at fork time).
  - Head: `2c43dc6`.
  - **Patch 1 — `PgCatalogContextProvider` for `Arc<T>` recursion fix.**
    The blanket `impl PgCatalogContextProvider for Arc<T>` in
    `datafusion-pg-catalog/src/pg_catalog/context.rs` called
    `self.roles()` / `self.role()`, which resolve to the trait method on
    `Arc<T>` itself rather than delegating to the inner `T`. Result:
    infinite recursion → `'tokio-rt-worker' has overflowed its stack;
    fatal runtime error: stack overflow, aborting` on any access to
    `pg_catalog.pg_roles` (or anything else that consults the context
    provider through an `Arc`). QuackGIS G2 probe finding; full repro in
    `crates/quackgis-server/probes/pg_catalog-readiness.md`.
    Fix: deref explicitly with `(**self).roles()` / `(**self).role(name)`.
  - **Diff:** 1 file, 7 insertions / 4 deletions. No logic changes —
    purely a method-resolution correction.
  - **Upstream plan:** file as a PR immediately. The bug is a hard crash
    affecting any user of `datafusion-pg-catalog` that passes an
    `Arc<T>` context provider (which is the documented usage pattern).
  - **Rebase expectations:** trivial — single-line fix in a stable file.

- **Planned additional patches (when needed):**
  - **G1** (M2): extend `arrow-pg` with a generalised type-extension hook so
    we can register SedonaDB geometry encodings without forking the encoding
    table.
  - **G3-BINARY** (M3 or M7): honour the `BINARY` cursor keyword in
    `CursorStatementHook` (currently always hex-text bytea — valid WKB but
    2× bandwidth overhead). Small change in `hooks/cursor.rs`.
  - **G3(b)** (M3): emit a matching `RowDescription` for FETCH through the
    extended protocol (currently `CursorStatementHook` stores a portal with
    no schema; tokio-postgres fails with `"DataRow field count does not
    match"`). Same file as G3-BINARY.
  - **G4** (M4, if probe finds a gap): portal suspension honoring
    `Execute.max_rows` for JDBC `setFetchSize`.

### Forks not yet needed

- `datafusion-ducklake` — comes in at **M1**. Will likely need a fork early
  for **G5** (UPDATE/DELETE) and **G7** (file/partition pruning). Stand up
  the fork when the M1 spike starts.

## Rebase hygiene

- Forks track upstream `main`/`master` on their default branch. Patches live
  on a `quackgis/*` working branch.
- Rebase `quackgis/*` onto new upstream tags at each QuackGIS milestone
  boundary; opportunistically mid-milestone if upstream lands a relevant fix.
- Every rebased commit must keep its original commit message + upstream-PR
  link (added when filed).
