# Contributing

QuackGIS is a thin integration layer over three upstream DataFusion projects
(datafusion-postgres, SedonaDB, datafusion-ducklake), consumed as **pinned
forks**. Several capabilities the design needs don't exist upstream yet — see
the gap ledger in [ROADMAP.md](./ROADMAP.md). Policy: **fork/vendor
preferred** — when a needed capability is missing, build it in our fork and
ship; upstream the patch opportunistically, never on the critical path. This
repo owns the PostGIS compatibility surface (geometry over the wire,
geometry_columns/spatial_ref_sys, client shims) and the glue.

Fork rules:

- Pin exact revisions (`[patch.crates-io]` or git rev); no floating branches.
- Minimal diffs; every patch listed in the fork's `DIVERGENCE.md` with its
  upstream PR link if one exists.
- Rebase forks onto upstream tags at milestone boundaries.

```sh
cargo build --release          # server binary
cargo test                     # unit + wire integration tests
cargo test -p quackgis-server  # the server crate only
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

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
