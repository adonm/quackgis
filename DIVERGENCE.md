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
- no protocol, authentication, handler, or message representation changes.

`QUACKGIS_PGWIRE_MAX_FRAME_BYTES` owns the release setting. The native workflow
sends only an oversized `CopyData` header, requires immediate connection closure,
and verifies zero published rows; the unit regression proves the five-byte header
does not grow its buffer. Retire this vendor as soon as upstream exposes an
equivalent pre-body configurable server limit, after the pgwire workflow, TLS,
COPY, and Arrow encoder gates pass on the released replacement.

## Retired forks

The following forks/vendors are no longer compiled or retained in the repository:

- `adonm/sedona-db` / Sedona SQL function crates;
- `datafusion-postgres` and `datafusion-pg-catalog`;
- `datafusion-ducklake`; and
- DataFusion itself.

Their historical patches remain available in Git history through commit
`81328a3` and earlier. New compatibility work belongs at the owned pgwire edge,
DuckDB SQL/macros, a narrowly scoped DuckDB extension, or upstream DuckDB/DuckLake.
