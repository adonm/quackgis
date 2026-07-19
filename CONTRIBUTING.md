# Contributing

QuackGIS owns a Rust pgwire/control edge over a pinned DuckDB/DuckLake/Spatial
native bundle. Missing compatibility belongs in bounded SQL rewrites/macros, the
Rust edge, an N0-owned QuackGIS extension, or a minimal upstream patch after the
earlier levels are exhausted. See [docs/NATIVE_BUNDLE.md](./docs/NATIVE_BUNDLE.md)
for source/patch/build rules,
[docs/UPSTREAM_ADOPTION.md](./docs/UPSTREAM_ADOPTION.md) for the mandatory
upstream-first/deletion workflow, and [DIVERGENCE.md](./DIVERGENCE.md) for current
divergence.

Fork rules:

- Pin native libraries/extensions by version and digest.
- Move DuckDB, DuckLake, Spatial, and QuackGIS-native revisions together through
  the bundle manifest; never use a floating branch in build or runtime policy.
- Keep vendored diffs minimal and documented in `DIVERGENCE.md`.
- Prefer upstream DuckDB/DuckLake behavior over parallel engine abstractions.
- Before native work, run `just native-upstream-check`, inspect moved release and
  compatible-branch refs, and prove why upstream cannot replace the change. Delete
  local overlap instead of preserving it behind the bundle.
- Keep upstream trees pristine before applying ordered, digest-pinned patches;
  do not rely on extension load order to override private C++ behavior.

```sh
mise install                   # pinned Rust/tool bootstrap
eval "$(mise activate bash)"   # use pinned tools/env in this shell
just --list                    # discover common tasks
just doctor                    # verify the pinned local toolchain
just smoke                     # smallest server + spatial query smoke
just ci                        # same fast gate used by GitHub Actions
just native-upstream-check     # opt-in live upstream/adoption review
just build                     # server binary
just test                      # unit tests + compile native integration targets
just check                     # fmt + clippy + tests
```

The repo uses `mise.toml` for tool/env management and `Justfile` as the stable
entrypoint for newcomers. Prefer an activated mise shell and reusable Justfile
recipes over ad hoc commands. For non-interactive/CI contexts, call the same
recipes through mise, for example `mise exec -- just ci`. Put new native/container
flows behind a Justfile recipe before adding them to docs or workflows.

Compatibility work is trace-driven: capture the SQL a client (QGIS, GeoServer,
OGR) actually sends, add it as a replay fixture, then fix. See
[ROADMAP.md](./ROADMAP.md) for forward outcomes and
[docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) for current frontiers.

Legacy PostgreSQL/DuckDB/C-extension assets are retired. Do not recreate or extend
that architecture. Read [docs/HISTORY.md](./docs/HISTORY.md) before mining old
commits; it identifies useful anchors, retained lessons, and current owners for
those ideas.

## Rules

- No silent geometry semantic changes.
- Validate at trust boundaries (wire input, WKB/EWKB) and fail closed.
- Follow the native → macro/rewrite → Rust edge → vectorized extension ladder.
- Do not implement row-wise spatial fallback in Rust.
- New compatibility requires a maintained client/workload fixture.
- Performance work records data, hardware, native versions, plans, memory, spill,
  and result correctness—not latency alone.
- Keep docs short — see [ROADMAP.md](./ROADMAP.md) for current priorities.
