# Fork, upgrade, and data compatibility policy

QuackGIS consumes young spatial/lakehouse dependencies through pinned versions and
tracked forks. This policy keeps rebases deliberate and release evidence
auditable.

## Fork policy

- Forked dependencies are declared in `Cargo.toml` as git branch dependencies.
- Every QuackGIS-owned fork patch needs an entry in `DIVERGENCE.md` describing:
  - upstream repository/branch;
  - QuackGIS branch/commit range;
  - capability or bug fixed;
  - upstreaming status;
  - tests/probes that fail without the patch.
- Reference checkouts live under ignored `.tmp/ref/*` through `just ref-init` and
  are never part of the Cargo workspace.
- In-tree `vendor/` is reserved for deep or long-lived divergence that cannot be
  reviewed as a small fork branch.

## Upgrade/rebase gates

Rebase fork branches or bump DataFusion/DuckLake/Sedona major lines only at a
deliberate checkpoint. Minimum gates before merging a rebaseline:

```sh
just check-fast
just preview-smoke
just layoutbench-sf0
just postgis-regress
just runtime-static-check
```

For storage or pgwire/catalog changes, also run the relevant Kind probes:

```sh
just kind-compatibility
just kind-alpha-smoke
```

Any intentional change to QPS, OLAP, bytes-scanned, file-group, or pass-rate
budgets should be backed by a run-stamped `metrics.json` artifact and summarized
in the release evidence record.

## Native DuckDB development artifacts

- `mise.toml` and `mise.lock` pin the DuckDB CLI used by local probes.
- `mise run duckdb-bootstrap` downloads the official Linux x86_64 `libduckdb`
  v1.5.4 archive, verifies its committed release SHA-256 and the extracted shared
  library SHA-256, and installs only into ignored `.tmp/duckdb`.
- DuckDB installs its signed, engine-version-matched `ducklake` and `spatial`
  extensions into an isolated repository-local home. The bootstrap records their
  resolved SHA-256 digests in `.tmp/duckdb/manifest.json`.
- Native library paths are operator/developer configuration, never client input.
  The ADBC boundary requires an absolute regular-file path and production remains
  fail-closed `LOAD` only. Network-backed `INSTALL` belongs only in the explicit
  bootstrap step.
- The ADBC open boundary re-hashes the selected library and requires both the
  committed SHA-256 and exact SQL runtime `version()` (`v1.5.4`) before claiming a
  data root or attaching DuckLake. A modified or mixed-version native runtime
  fails before creating the authority marker or data directory.
- This developer path currently supports Linux x86_64. D4 still requires a
  published platform matrix, immutable production artifacts, upgrade/mixed-version
  refusal, and clean-room deployment evidence.
- `scripts/prepare_duckdb_runtime.py` independently checks committed SHA-256 values
  for the shared library and both extensions before constructing an image context.
  `deploy/Containerfile.duckdb-runtime` also pins the base image digest and has an
  offline `--network none` load smoke. Update the bootstrap and runtime checksums
  together only after the real ADBC, spatial, compatibility, and recovery gates.

## Data/catalog compatibility

- SQLite/local and PostgreSQL/S3 profiles are both maintained storage contracts.
- Do not mix DataFusion major versions in one milestone.
- DuckLake catalog/object data should remain forward-compatible with the official
  DuckLake 1.0+ direction where practical; QuackGIS-specific behavior must be
  documented when it depends on forked APIs.
- Native delete-file or partial-rewrite DML must not be enabled from independent
  per-file metadata commits. The vendored fork now provides atomic table
  mutations for delete files, appended data files, and selected file retirement;
  native autocommit `UPDATE` is routed through that single-snapshot API, and
  bucket compaction must follow the same rule. See `docs/NATIVE_DML_FORK_PLAN.md`.
- `docs/DUCKLAKE_ALIGNMENT.md` is the compatibility ledger for fork-backed or
  upstream-sensitive DuckLake behavior: current implementation, upstream target,
  interop gate, and migration trigger.
- Before claiming upgrade compatibility between releases, validate an existing
  DuckLake catalog/data prefix with the new binary: table discovery, representative
  counts/bboxes, spatial reads, `geometry_columns`, and compaction on a copied
  environment.
- Never test destructive migration behavior on the live object-store prefix; copy
  or snapshot the catalog + object prefix first.

## Release checklist hook

For every tag/main artifact build, attach or reference:

- `release-evidence-<version>.json` from the artifact workflow;
- matching compatibility/storage `metrics.json` artifacts for the source SHA;
- matching `metrics-dashboard.md` summaries when metrics artifacts exist;
- any dependency rebase notes and `DIVERGENCE.md` changes;
- updated `docs/DUCKLAKE_ALIGNMENT.md` entries for storage behavior changes;
- known data/catalog compatibility limits for the release.
