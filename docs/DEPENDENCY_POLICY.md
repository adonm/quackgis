# Dependency policy

## Engine and storage

- DuckDB is the sole query/spatial engine.
- Official DuckLake is the sole writer for new durable storage.
- DuckDB, `spatial`, `ducklake`, and any QuackGIS-native extension move as one N0
  bundle behind native storage, pgwire, independent-reopen, runtime-package,
  recovery, rollback, and upgrade gates.
- Runtime artifacts are checksum-pinned and preinstalled; production does not
  download extensions.
- The current read-only DuckLake identity patch documented in
  `PINNED_DUCKLAKE.md` remains supported until N0 reproduces its evidence. N0
  generalizes that policy to full DuckDB/DuckLake/Spatial/QuackGIS source commits,
  ordered patch digests, build options, toolchain, licenses/SBOM, and artifacts as
  specified in `NATIVE_BUNDLE.md`.
- Upstream sources are prepared into ignored workspace-local checkouts. The main
  repository tracks manifests, patch queues, owned extension source, tests, and
  accepted digests rather than full Git histories, generated CRS source, build
  outputs, or vcpkg caches. Offline source archives may be release artifacts.
- DataFusion, SedonaDB, forked DuckLake writers, and auxiliary engines require a
  new architecture decision and are not acceptable transitive conveniences.
- Upstream roadmap adoption follows `DUCKDB_ROADMAP_ALIGNMENT.md`: N0 evaluates
  only released supported candidates and treats calendar/nightly entries as
  evidence inputs; do not publish Local 1.0 on an unsupported engine line. Async
  client I/O and C/Rust extension APIs
  may replace current code only after equivalence and upgrade gates pass. Quack or
  another remote engine protocol is watch-only and requires an explicit direction
  change plus complete attached-DuckLake data-plane evidence.

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
frontend frame trust boundary and expose a concrete no-auth startup-parameter
provider hook, but may not add protocol or authentication policy.

The REST interface uses two unmodified crates from
`joshburgess/pg-rest-server` at one full Git revision recorded in
`DIVERGENCE.md` and `Cargo.lock`. They are extension inputs, not an authority for
storage, authentication, or PostgreSQL emulation. Revision changes require the
same license/source review and both REST gates.

New vendor/fork acceptance requires:

1. an upstream gap blocking a maintained release requirement;
2. a smaller native/macro/Rust-edge solution ruled out;
3. minimal documented divergence;
4. tests and upgrade ownership; and
5. a deletion/upstream plan.

Native patches additionally require one N0 patch-queue entry, an exact base
commit, patch digest, clean application check, upstream test coverage,
unmodified-versus-patched differential evidence, artifact provenance, and
recovery/upgrade ownership. Loading a later extension does not count as overriding
private C++ behavior; changed writer/type/planner semantics require a source patch
or accepted upstream hook.

The DuckLake identity patch satisfies the current pre-N0 conditions:
exact source/submodule commits, tracked patch, build inputs, accepted artifact
digest, focused tests, trust boundary, upgrade ownership, and deletion plan are
recorded in `PINNED_DUCKLAKE.md` and `DIVERGENCE.md`. It does not modify or replace
DuckLake's writer path. N0 must ingest or delete this patch without weakening its
gates. Upstream adoption remains the preferred deletion path.

## Upgrade evidence

An engine/storage upgrade is complete only when exact artifact versions/digests
are recorded and these gates pass:

- N0 clean preparation, patch application, source/toolchain/license/SBOM checks,
  and mixed-bundle refusal;
- Rust unit/all-target tests and clippy;
- pinned native ADBC storage workflow;
- real pgwire workflow and spatial corpus;
- independent official DuckDB reopen;
- runtime image load-only/static checks;
- catalog/data backup, reopen, and rollback evidence once Local 1.0 supports it;
- classic/PEG parser acceptance and denial parity while both modes are available; and
- deletion review for compatibility, pruning, transport, and extension code
  superseded by the candidate bundle.
