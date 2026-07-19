# Fork divergence ledger

## Active vendored code

### `vendor/arrow-pg`

QuackGIS retains a small Arrow-to-pgwire encoder derived from
`datafusion-contrib/datafusion-postgres`'s `arrow-pg` crate.

Local ownership:

- DataFusion dataframe support and its optional dependency are removed;
- GeoArrow/PostGIS optional implementation branches are removed;
- binary Arrow fields may advertise QuackGIS geometry/geography sentinel OIDs;
- `quackgis.spatial_family` field metadata takes precedence over conservative
  column-name fallback;
- WKB/EWKB remains raw binary in PostgreSQL binary format and hex in text format;
- generated properties prove geometry sentinel payload identity and fixed-size
  binary/null encoding; Float16, UInt32 OID aliases, and advertised Float16/fixed
  binary lists have parity regressions; unsupported list layouts fail during
  schema mapping;
- structurally proven catalog projections carry explicit Arrow metadata for
  PostgreSQL OID, `name`, `name[]`, internal `char`, and information-schema
  `varchar` wire types; ordinary aliases retain their native scalar type and
  value;
- invalid JSON fails closed instead of silently becoming JSON `null`; and
- nested struct failures propagate without panicking; and
- the crate's tests are an explicit `just ci` prerequisite.

This encoder should eventually become a QuackGIS-owned crate with focused
property/fuzz coverage for every advertised Arrow type. Generated WKB and
fixed-binary properties plus focused scalar/list/struct parity regressions are
maintained; broader generated temporal, decimal, dictionary, and nested-type
coverage remains open. Until then, Arrow and pgwire versions are pinned together
with the server.

### `vendor/pgwire`

QuackGIS pins a narrow copy of upstream `pgwire` 0.40.4 because the released
server codec validates type-specific packet limits before body decoding but gives
`CopyData` a nearly 1 GiB ceiling and exposes no deployment-level override.

Local divergence is limited to:

- a configurable maximum declared frontend-message length in the server codec;
- `process_socket_with_frontend_limit`, which applies the limit before plaintext
  or TLS application-frame body reads;
- a header-only validator used by the focused no-allocation regression; and
- a `NoopStartupHandler` hook returning the concrete default server-parameter
  provider, allowing trust and edge-preauthenticated QuackGIS sessions to
  advertise the same PostgreSQL 18 profile as SCRAM without changing
  authentication messages or decisions.

`QUACKGIS_PGWIRE_MAX_FRAME_BYTES` owns the release setting. The native workflow
sends only an oversized `CopyData` header, requires immediate connection closure,
and verifies zero published rows; the unit regression proves the five-byte header
does not grow its buffer. Retire this vendor as soon as upstream exposes an
equivalent pre-body configurable server limit, after the pgwire workflow, TLS,
COPY, and Arrow encoder gates pass on the released replacement.

### `joshburgess/pg-rest-server`

`quackgis-rest` extends the upstream `pg-query-engine` and
`pg-schema-cache-types` crates at exact Git revision
`b7915d3c3361f0fee45de6e292e62f6f6186375f` (MIT OR Apache-2.0). The source is
not copied or silently patched. QuackGIS owns a separate read-only backend because
upstream's data paths assume PostgreSQL catalogs, roles/RLS, LISTEN/NOTIFY, and
PostgreSQL-side JSON.

Current local divergence is limited to DuckDB `information_schema` discovery,
omission of PostgreSQL role switching, explicit text parameter typing for pgwire,
and exact generated-function adaptation from `json_agg`/`to_jsonb` to DuckDB
`json_group_array`/`to_json`. The native compatibility smoke crosses real pgwire
and fails closed for mutations. The planned PostgreSQL 18 catalog/RBAC work moves
schema and authorization semantics into QuackGIS so this backend can become a
normal authenticator/role-switching pgwire client; it does not make the upstream
PostgreSQL execution backends QuackGIS dependencies. Upstream updates require
reviewing the parser, SQL generator, schema types, license, and compatibility
cases before changing the pinned revision.

## Active source patches

N0 will move native divergence into ordered DuckDB/DuckLake/Spatial patch queues
under the one bundle manifest described in `docs/NATIVE_BUNDLE.md`. The entries
below describe current evidence until N0 reproduces or deletes them; source trees
and unexplained edits are not vendored into QuackGIS.

### DuckLake column identity

QuackGIS applies `patches/ducklake/ducklake-column-info.patch` to an exact
DuckLake `v1.5-variegata` commit targeting exact DuckDB `v1.5.4`. The
read-only `ducklake_column_info(catalog)` function exposes current top-level
base-table schema/table/column identities from the committed snapshot; it does
not change DuckLake metadata or data writes. `native/bundle.json` and
`patches/ducklake/series.json` record the upstream/core trees, patch/result tree,
legacy accepted-artifact build provenance, common candidate toolchain, platform,
and accepted artifact digest. `scripts/build_pinned_ducklake.py` now consumes
that common authority while preserving the current artifact reproduction gate.
Exact behavior, trust boundaries, and lifecycle evidence are documented in
`docs/PINNED_DUCKLAKE.md`. `native/upstream-review.json` records that both the
selected DuckLake 1.5 tip and reviewed `main` still lack an equivalent public
column-identity function; every moved ref requires this retain/delete decision to
be revisited. The generated license inventory records the local patch itself as
`NOASSERTION`; explicit declared/concluded-license review is release-blocking and
must not be conflated with upstream DuckLake's MIT license.

Local 1.0 packages the accepted unsigned binary and passes its absolute immutable
path plus exact SHA-256 to the server. This is a long-term support obligation:
each bundle candidate must rebuild and pass DuckLake function, QuackGIS identity,
storage, pgwire, packaging, recovery, and rollback gates. Retire the patch and
unsigned-extension policy when an official version-matched DuckLake exposes the
same API and passes those gates.

N0 must either ingest this patch as an exact ordered bundle patch or replace it
with a released public API. CRS work first qualifies official DuckDB/Spatial and
DuckLake type persistence; key work may add a DuckLake lifecycle hook only after
Q0 defines enforced semantics. Neither is added to this ledger merely as a design
idea.

## Retired forks

The following forks/vendors are no longer compiled or retained in the repository:

- `adonm/sedona-db` / Sedona SQL function crates;
- `datafusion-postgres` and `datafusion-pg-catalog`;
- `datafusion-ducklake`; and
- DataFusion itself.

Their historical patches remain available in Git history through commit
`81328a3` and earlier. New compatibility work follows the native → SQL/rewrite →
Rust edge → N0 QuackGIS extension → minimal upstream patch ladder; official
DuckLake remains the only user-data writer.
