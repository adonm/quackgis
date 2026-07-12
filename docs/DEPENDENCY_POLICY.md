# Dependency policy

## Engine and storage

- DuckDB is the sole query/spatial engine.
- Official DuckLake is the sole writer for new durable storage.
- DuckDB, `spatial`, and `ducklake` versions move together behind native storage,
  pgwire, independent-reopen, runtime-package, and upgrade gates.
- Runtime artifacts are checksum-pinned and preinstalled; production does not
  download extensions.
- DataFusion, SedonaDB, forked DuckLake writers, and auxiliary engines require a
  new architecture decision and are not acceptable transitive conveniences.

## Rust dependencies

- Prefer small direct crates and explicit features.
- Keep Arrow major versions aligned with ADBC and the encoder.
- Pin protocol/parser versions when AST or handler APIs are structurally matched.
- Run `cargo test --workspace --all-targets` and clippy with `-D warnings` for
  dependency upgrades.
- Audit `Cargo.lock` for accidental engine, native, TLS, and network additions.

## Vendored code

`vendor/arrow-pg` and the narrowly patched `vendor/pgwire` are the active vendors.
Their divergences are documented in `DIVERGENCE.md`. The encoder may map and
encode Arrow/PostgreSQL fields and rows; it may not regain DataFusion, catalog,
planner, or GeoArrow engine responsibilities. The pgwire patch may enforce the
frontend frame trust boundary but may not add protocol or authentication policy.

New vendor/fork acceptance requires:

1. an upstream gap blocking a maintained release requirement;
2. a smaller native/macro/Rust-edge solution ruled out;
3. minimal documented divergence;
4. tests and upgrade ownership; and
5. a deletion/upstream plan.

## Upgrade evidence

An engine/storage upgrade is complete only when exact artifact versions/digests
are recorded and these gates pass:

- Rust unit/all-target tests and clippy;
- pinned native ADBC storage workflow;
- real pgwire workflow and spatial corpus;
- independent official DuckDB reopen;
- runtime image load-only/static checks; and
- catalog/data backup, reopen, and rollback evidence once Local 1.0 supports it.
