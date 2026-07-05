# G2 introspection probe — pg_catalog readiness

Date: 2026-07-05. Method: ran a battery of typical QGIS/GeoServer-style
introspection queries against a freshly-started `quackgis-server` (M0 build
`b5cbeff`+`99e3a7d`, on `datafusion-postgres` master + SedonaDB `quackgis/df53`
fork). Captured what works, what errors cleanly, and what crashes.

## Works ✅

| Query | Result |
|---|---|
| `SELECT current_database()` | `datafusion` |
| `SELECT version()` | `Apache DataFusion 53.1.0, x86_64 on linux` |
| `SELECT count(*) FROM pg_catalog.pg_namespace` | 2 |
| `SELECT count(*) FROM pg_catalog.pg_database` | 2 |
| `SELECT count(*) FROM pg_catalog.pg_class` | 69 |
| `SELECT count(*) FROM pg_catalog.pg_type` | 617 |
| `SELECT count(*) FROM pg_catalog.pg_attribute` | 684 |
| `SELECT count(*) FROM pg_catalog.pg_index` | 164 |
| `SELECT count(*) FROM pg_catalog.pg_proc` | 3330 |
| `SELECT count(*) FROM information_schema.tables` | 76 |
| `SELECT count(*) FROM information_schema.columns` | 684 |
| `SELECT count(*) FROM information_schema.schemata` | 2 |
| `SET client_min_messages='warning'` | OK |
| `SET application_name='qgis'` then `SHOW application_name` | `qgis` — state tracked |
| `SET enable_seqscan=off` | OK |
| `CREATE TABLE x (id INT); INSERT...; SELECT count(*)...; DROP TABLE x;` | full DDL+DML roundtrip works in-memory — not DuckLake-backed |

## Errors cleanly ✅ (failure is a normal query error, not a crash)

| Query | Error |
|---|---|
| `SELECT pg_postmaster_start_time()` | `Invalid function 'pg_postmaster_start_time'` |
| `SELECT postgis_version()` | `Invalid function 'postgis_version'` (expected — G2/M2 work) |
| `SELECT count(*) FROM geometry_columns` | `table 'datafusion.public.geometry_columns' not found` (expected — M2 work) |
| `SELECT count(*) FROM spatial_ref_sys` | same (M2 work) |
| `SHOW client_min_messages` | returns empty (SetShowHook only tracks a subset of GUCs) |
| `SELECT current_user()` | `sql parser error: Expected: end of statement, found: ()` — likely reserved-word issue; try `SELECT current_user` without parens |

## Crashes 🔴 → ✅ FIXED

| Query | Effect | Status |
|---|---|---|
| `SELECT count(*) FROM pg_catalog.pg_roles` | **Stack overflow** in pg_catalog view resolution. Server process aborts with `thread 'tokio-rt-worker' has overflowed its stack; fatal runtime error: stack overflow, aborting`. Connection dropped; no clean error to the client. | **Fixed** in `adonm/datafusion-postgres@quackgis/fixes` (commit `2c43dc6`). Root cause: blanket `impl PgCatalogContextProvider for Arc<T>` self-recursed (`self.roles()` resolved to the Arc impl rather than the inner T). Fix: `(**self).roles()`. Regression test `pg_roles_does_not_crash` added to the wire suite. Upstream PR candidate. |

## Implications

### For M2 (PostGIS SQL surface)
- PostGIS-compat shims need to register: `postgis_version()`, `postgis_lib_version()`, `geometry_columns` view, `spatial_ref_sys` table. None exist at M0 — confirmed clean-fail, not a crash.

### For M3 (QGIS read path) — G2 scope sharper
- The common five introspection tables (pg_namespace, pg_database, pg_class, pg_type, pg_attribute) work, plus pg_index, pg_proc. **No fork-bump of datafusion-pg-catalog needed for the QGIS baseline.**
- **`pg_roles` is a hard blocker** until fixed: it's a stack overflow in upstream `datafusion-pg-catalog`, not a missing feature. GeoServer queries pg_roles to enumerate database roles; QGIS may not. Either we contribute the fix upstream (preferred once we have a stack trace + repro) or skip/redirect the view in our fork.
- `pg_postmaster_start_time()` and a handful of metadata functions need to be registered. Likely a small fork addition to datafusion-pg-catalog, or quackgis-side UDFs.

### For G4 (extended-protocol fetch)
- The pg_roles crash plus the G3(b) extended-FETCH bug suggest `datafusion-postgres`'s cursor + complex-view paths need stress testing before GeoServer. M3 work should include a pgjdbc-style stress harness.

## Action items

1. ~~**`pg_roles` stack overflow** — capture a stack trace (build with debug
   symbols, RUST_BACKTRACE=full, rerun). File issue/PR upstream with repro.~~
   **DONE.** Root cause identified by source-reading; fix landed in
   `adonm/datafusion-postgres@quackgis/fixes` (commit `2c43dc6`). Upstream
   PR candidate.
2. **`pg_postmaster_start_time()` and friends** — enumerate the metadata
   functions QGIS/GeoServer actually call and register them as no-op or
   constant-valued UDFs in the quackgis-server crate (low cost, no fork).
3. **PostGIS surface** — M2 work, not blocked by this probe.
