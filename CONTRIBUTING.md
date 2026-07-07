# Contributing

QuackGIS is a thin integration layer over three upstream DataFusion projects
(datafusion-postgres, SedonaDB, datafusion-ducklake), consumed as **tracked fork
branches**. Several capabilities the design needs don't exist upstream yet — see
the gap ledger in [ROADMAP.md](./ROADMAP.md). Policy: **fork/vendor
preferred** — when a needed capability is missing, build it in our fork and
ship; upstream the patch opportunistically, never on the critical path. This
repo owns the PostGIS compatibility surface (geometry over the wire,
geometry_columns/spatial_ref_sys, client shims) and the glue.

Fork rules:

- Track upstream heads through named `quackgis/*` fork branches; no silent floating dependency changes outside commits.
- Minimal diffs; every patch listed in the fork's `DIVERGENCE.md` with its
  upstream PR link if one exists.
- Rebase forks onto upstream tags at milestone boundaries.

```sh
mise install                   # pinned Rust/tool bootstrap
eval "$(mise activate bash)"   # use pinned tools/env in this shell
just --list                    # discover common tasks
just doctor                    # verify the pinned local toolchain
just smoke                     # smallest server + spatial query smoke
just demo-local                # local host demo with stable public.demo_* layers
just demo-kind                 # local Kind demo with stable public.demo_* layers
just ci                        # same fast gate used by GitHub Actions
just build                     # server binary
just test                      # unit + wire integration tests
just martin-sql                # Martin-generated SQL compatibility
just check                     # fmt + clippy + tests
```

The repo uses `mise.toml` for tool/env management and `Justfile` as the stable
entrypoint for newcomers. Prefer an activated mise shell and reusable Justfile
recipes over ad hoc commands. For non-interactive/CI contexts, call the same
recipes through mise, for example `mise exec -- just ci` or
`mise exec -- just kind-compatibility`. Put new cargo/kubectl/container flows
behind a Justfile recipe before adding them to docs or workflows.

Compatibility work is trace-driven: capture the SQL a client (QGIS, GeoServer,
OGR) actually sends, add it as a replay fixture, then fix. See
[ROADMAP.md](./ROADMAP.md) for the current milestone and gates.

Note: legacy v0.1 assets (DuckDB extension in `src/`, `container/init.d/`,
`vendor/`) are being retired at M0 — don't extend them; see ROADMAP.md
"Retired v0.1 assets".

## Rules

- No silent geometry semantic changes.
- Validate at trust boundaries (wire input, WKB/EWKB) and fail closed.
- Rust-first owned code; fork/vendor when a needed capability is missing
  upstream (follow the fork rules above); upstream opportunistically.
- Keep docs short — see [ROADMAP.md](./ROADMAP.md) for current priorities.
