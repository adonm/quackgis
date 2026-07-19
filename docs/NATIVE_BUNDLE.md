# Native bundle

This document owns the target source, patch, build, trust, and upgrade boundary for
DuckDB, DuckLake, Spatial, and QuackGIS-native code. It describes the N0 workstream
in [ROADMAP.md](../ROADMAP.md). Current executable evidence remains in
[ROADMAP_STATUS.md](./ROADMAP_STATUS.md).

## Purpose

QuackGIS needs one reproducible native unit rather than three independently moving
artifacts and an implicit patch process. The bundle will:

- select one supported DuckDB core commit and ABI;
- select compatible exact DuckLake and Spatial commits;
- apply ordered, digest-pinned QuackGIS patch queues where public APIs are
  insufficient;
- build a separate QuackGIS extension for additive native behavior;
- run upstream and QuackGIS tests against one central DuckDB checkout;
- produce immutable artifacts, licenses, source provenance, and an SBOM; and
- upgrade, reopen, restore, or roll back as one reviewed unit.

“Latest” is never a runtime selector. An update resolves candidate commits, freezes
them in the bundle manifest, runs the complete matrix, and accepts exact artifact
digests. Production does not follow branches or download extensions.

## Current and target state

The current runtime already proves part of this model:

- DuckDB 1.5.4 and the official Spatial artifact are checksum-pinned;
- `patches/ducklake/pin.json` pins one DuckLake source, DuckDB submodule, patch,
  toolchain, and accepted artifact;
- `scripts/build_pinned_ducklake.py` clones, checks, patches, builds, and tests that
  exact DuckLake source; and
- runtime assembly verifies immutable native artifacts and performs no online
  extension installation.

N0 generalizes that DuckLake-only lane. Until N0 closes, the existing pin and
commands remain the current supported development path; proposed bundle commands
or manifests are not current evidence.

## Source layout

Upstream source trees and build products remain ignored workspace inputs under
`.tmp/ref/`. QuackGIS tracks only manifests, patch queues, owned extension code,
build orchestration, tests, licenses, and accepted artifact digests:

```text
native/
  bundle.json
  extension_config.cmake
  quackgis/
    CMakeLists.txt
    src/
    test/sql/
patches/
  duckdb/
    series.json
  ducklake/
    series.json
  spatial/
    series.json
scripts/
  prepare_native_bundle.py
  build_native_bundle.py
  package_native_bundle.py
```

A release may publish checksum-pinned source archives for offline rebuilding. Full
upstream Git histories, generated build trees, vcpkg caches, and large test data do
not belong in the main repository.

## Bundle authority

One machine-readable manifest records at least:

- bundle format and bundle ID;
- full DuckDB source commit and release status;
- full DuckLake and Spatial source commits;
- the DuckDB commit each extension targets;
- ordered patch paths and SHA-256 digests for DuckDB, DuckLake, and Spatial;
- a digest of QuackGIS-owned extension source;
- vcpkg, compiler, CMake, Ninja, platform, and build options;
- whether Spatial networking and optional modules are enabled;
- expected upstream and QuackGIS test groups;
- accepted library and extension artifact SHA-256 values; and
- license/SBOM outputs.

The preparer fails when a commit is not exact, extension/core pins disagree without
an explicit reviewed exception, a patch hash drifts, a patch does not apply, a
reused checkout is dirty or unrecognized, or an accepted artifact differs.

## Build model

One central DuckDB checkout builds every extension through DuckDB's out-of-tree
extension mechanism. Prepared local source directories are passed through
`duckdb_extension_load(... SOURCE_DIR ...)`; they are not separately built against
different embedded DuckDB submodules. A merged pinned vcpkg manifest supplies the
combined DuckLake and Spatial native dependencies.

The central build emits candidate loadable DuckLake, Spatial, and QuackGIS
artifacts plus the exact DuckDB library/CLI used for tests. A release may select
an exact vendor-built signed extension instead of its local candidate only when
the manifest binds that binary to the qualified source/ABI and all runtime gates
pass against those exact bytes. Static linking may be evaluated separately but
must not create a second untested runtime. Build configuration, generated source,
and optional-module choices are part of the bundle identity.

## Layering and patch rules

Native changes follow this order:

1. **Official behavior:** use released DuckDB, DuckLake, or Spatial semantics
   unchanged when they pass the product oracle.
2. **QuackGIS extension:** add functions, table functions, native metadata
   extraction, and validation primitives through supported extension APIs.
3. **Extension patch:** patch DuckLake or Spatial only when the required behavior
   needs private extension state or a missing lifecycle hook.
4. **DuckDB core patch:** patch core only when no supported extension boundary can
   provide the required semantics.

Loading another extension does not override arbitrary C++ methods. Duplicate SQL
function names or load order are not an override contract. Changed writer,
transaction, type, planner, or registration behavior uses an explicit source patch
or an accepted upstream hook.

Upstream trees remain pristine before patch application. QuackGIS changes are
reviewable ordered patches or owned files, never unexplained edits in a copied
source tree. Every patch has:

- one maintained requirement;
- focused upstream-style tests;
- QuackGIS integration evidence;
- an ABI/upgrade owner;
- a divergence-ledger entry; and
- an upstream or deletion plan.

A patch conflict during a candidate update is a review gate, not something the
builder resolves automatically.

## Ownership boundaries

Official DuckLake remains the only user-table metadata/data writer. A QuackGIS
extension may expose compatibility metadata or validation primitives but may not
silently become an independent catalog, snapshot writer, or storage format.
Changing DuckLake write or commit behavior requires an explicit architecture
record, transaction/recovery tests, independent-reader evidence, and a narrowly
scoped patch or upstream hook.

Spatial remains the exact computation and CRS engine. PostgreSQL catalog mapping,
OID/SRID translation, pgwire types, and client policy remain QuackGIS concerns.
Spatial is patched only for an engine semantic defect, not to implement
`spatial_ref_sys` presentation.

Rust continues to own pgwire, authentication, authorization, SQL admission, COPY
framing, resource policy, and operational control. Native code does not duplicate
those boundaries.

## Runtime trust boundary

An official extension stays signed only when the selected runtime member is the
exact vendor-built signed binary. Every locally built extension is project-owned
native code even when its source has no QuackGIS patch. Every project-built or
patched DuckLake, Spatial, or QuackGIS extension must have:

- an absolute non-symlink immutable path;
- a compile-time accepted SHA-256 matched before DuckDB initialization;
- root-owned read-only image placement;
- bootstrap-only loading;
- client `LOAD` and `INSTALL` denial; and
- source, patch, toolchain, artifact, license, and SBOM identity in the runtime
  manifest.

If unsigned loading is enabled, it is enabled only after the complete selected
bundle policy validates. Startup does not accept an arbitrary unsigned extension
merely because another bundle member is patched.

## Qualification matrix

A candidate bundle cannot become the release bundle until it passes:

- clean source preparation and deterministic patch application;
- upstream DuckLake and Spatial focused/complete test groups;
- QuackGIS native storage, identity, pgwire, REST, migration, and spatial suites;
- classic/PEG parser allow/deny/rewrite equivalence where both modes exist;
- independent stock, version-matched DuckDB reopen;
- old-bundle catalog/data reopen followed by new writes;
- backup/restore and explicit rollback to the prior bundle;
- runtime image offline/load-only checks and mixed-bundle refusal;
- named psql, psycopg, OGR, and QGIS package gates;
- M1/M2 resource profiles and M4 exact plan/scan profiles; and
- patch deletion review for behavior now provided upstream.

The matrix compares the unmodified candidate with the patched bundle so each
behavioral difference is intentional and named.

## Upgrade flow

A bundle update is one reviewable change:

1. resolve supported candidate commits from one DuckDB release line;
2. freeze full source commits and toolchain inputs;
3. apply every existing patch without automatic conflict resolution;
4. run upstream tests before QuackGIS integration tests;
5. build and hash all native artifacts;
6. run reopen, recovery, client, performance, and package gates;
7. remove patches superseded by upstream behavior;
8. update the runtime manifest, divergence ledger, compatibility status, and
   release evidence; and
9. retain the prior accepted bundle and rollback procedure until the new bundle
   closes its soak/upgrade gates.

## Work after N0

N0 deliberately precedes the two unresolved compatibility authorities. S0 and
Q0 are independent after N0; S0 is the first upstream-adoption target, while Q0's
client-scope decision may proceed in parallel. Neither widens the default Local
1.0 contract until its own decision and gates pass:

1. **S0 authoritative CRS:** qualify released CRS-aware `GEOMETRY` types,
   `ST_CRS`/`ST_SetCRS`/`ST_Transform`, the official CRS catalog, DuckLake
   persistence, independent reopen, and PostgreSQL/QGIS/OGR projection. Patch
   DuckLake only if official CRS type metadata is lost; do not fork Spatial for a
   catalog-presentation problem.
2. **Q0 validated keys:** decide the required direct-table/creation client surface,
   then implement only keys whose NOT NULL/uniqueness semantics are validated on
   every supported write and recovery path. Unenforced declarations do not become
   PostgreSQL key claims. Any DuckLake writer hook follows the N0 patch and trust
   rules.

This ordering prevents key and CRS experiments from being built on another
one-off native artifact lane.
