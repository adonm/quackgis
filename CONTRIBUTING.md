# Contributing

QuackGIS owns a Rust pgwire/control edge over pinned DuckDB Spatial and official
DuckLake. Missing compatibility belongs in bounded SQL rewrites/macros, the Rust
edge, or a narrowly scoped DuckDB extension after native SQL is exhausted. See
[DIVERGENCE.md](./DIVERGENCE.md) for the one retained Arrow encoder fork.

Fork rules:

- Pin native libraries/extensions by version and digest.
- Keep vendored diffs minimal and documented in `DIVERGENCE.md`.
- Prefer upstream DuckDB/DuckLake behavior over parallel engine abstractions.

```sh
mise install                   # pinned Rust/tool bootstrap
eval "$(mise activate bash)"   # use pinned tools/env in this shell
just --list                    # discover common tasks
just doctor                    # verify the pinned local toolchain
just smoke                     # smallest server + spatial query smoke
just ci                        # same fast gate used by GitHub Actions
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
