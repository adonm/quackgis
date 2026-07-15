# QuackGIS

QuackGIS is a PostGIS-oriented spatial lakehouse server built around DuckDB
Spatial and official DuckLake, exposed through an owned Rust PostgreSQL wire edge.

```text
PostgreSQL / GIS / application clients
                  │ pgwire
                  ▼
Rust edge: protocol · TLS/SCRAM · roles/catalogs · policy · PostGIS compatibility
                  │ Arrow / ADBC
                  ▼
DuckDB: SQL · Spatial · official DuckLake · Parquet
```

An optional authenticated, read-only [`quackgis-rest`](docs/REST_API.md) sidecar
extends a pinned `pg-rest-server` query contract and exposes a load-balanceable
PostgREST-style HTTP read interface through the same pgwire boundary.

The product direction makes PostgreSQL 18 catalog, role, privilege, and session
semantics first-class QuackGIS capabilities. The REST edge will become a normal
authenticator/role-switching pgwire client of the packaged tiny iroh bridge and
generate role-aware OpenAPI from that common contract; this target is planned,
not part of the current preview.

There is no PostgreSQL, DataFusion, or Sedona query engine. DuckDB is the sole
planner/executor and official DuckLake is the sole writer for new storage.

## Status

**Local developer preview.** The maintained native workflow proves:

- official local DuckLake create/write/snapshot inspection/reopen;
- bounded simple and extended pgwire;
- parameterized reads and mutations;
- independent transactions and portal paging;
- PostgreSQL text COPY for maintained scalar/WKB types;
- maintained client settings, `public` schema mapping, and quoted COPY targets;
- optional reserved bbox columns validated and maintained by DuckDB during COPY,
  with conservative exact-rechecked candidate injection for bounded one-table
  `ST_Intersects` shapes;
- explicitly authorized adjacent-file compaction through a server-owned call;
- SCRAM and parsed read/write table policy; and
- 43 curated spatial cases using original PostGIS spellings through DuckDB native
  functions or bounded QuackGIS rewrites/macros, with stable `0A000` behavior for
  all 15 classified gaps.

Important limits:

- query results stream one native Arrow batch at a time; COPY incrementally builds
  bounded Arrow batches and publishes atomically through session-local staging;
- query cancellation/deadlines, global and reader/writer-class admission, DuckDB
  resource controls, and query lifecycle/batch metrics are implemented; result
  batches fail closed at a configured byte ceiling, and native calls use a fixed
  worker budget with a reserved cancellation slot; clean 1M/10M BIGINT-stream,
  1M nullable wide-result, 100-cancel, and mixed-class admission evidence passes,
  a clean eligible 50M transport reference passes the pgwire overhead budget, and
  the clean 10M COPY reference passes RSS/throughput/atomicity budgets; writes are
  cancellable before the non-cancellable commit boundary with explicit
  rollback/quarantine outcomes;
- broad PostgreSQL catalogs and named GIS-client parity are incomplete;
- immutable roles, membership validation, session/effective identity, and role
  switching are implemented when a role file is configured; configured schema
  USAGE and table ownership/SELECT/INSERT/UPDATE/DELETE/MAINTAIN now enforce
  statements behind the legacy allowlist ceiling; relational `pg_roles` and
  `pg_auth_members` project stable identities/options without credentials, and
  bounded `pg_has_role`/schema/table/column inquiry uses the same role decisions;
  role-filtered `information_schema` schema/table/column and portable table/column
  grant views use those decisions and PostgreSQL 18 wire types; role-aware OpenAPI
  remains open; bounded transaction-local `request.jwt.claims` context is
  implemented for the future authenticator flow; and
- remote/shared catalog and object-storage profiles fail closed.

TLS remains optional for local development. Set `QUACKGIS_TLS_MODE=required` with
`QUACKGIS_TLS_CERT` and `QUACKGIS_TLS_KEY` to fail closed on plaintext startup.
The actual-process TLS profile verifies encrypted SCRAM, hostname/trust checking,
plaintext denial, and restart-based certificate/password rotation; packaged Kind
rotation remains open.

The maintained Rust pgwire client resolves PostgreSQL 18 profile/QGIS-required
built-in types, their array partners, collations, and spatial sentinels through
process-local relational compatibility views. Explicit and implicit `pg_catalog`
lookup, reference integrity, PostgreSQL result types, and RowDescription text,
binary, and NULL WKB transport pass. User-object catalogs are not implemented;
traced unsupported relations fail closed, so this is not yet QGIS/OGR discovery
evidence against QuackGIS.

When `QUACKGIS_METRICS_PORT` is configured, the same loopback HTTP listener serves
`/healthz`, startup/drain-aware readiness at `/readyz`, and Prometheus data at
`/metrics`.

See [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) for exact evidence.

## Quick start

```sh
mise install
mise run duckdb-bootstrap
mise exec -- just smoke
```

Run the server:

```sh
mkdir -p .tmp/duckdb-server
mise exec -- cargo run -p quackgis-server -- \
  --catalog-path=.tmp/duckdb-server/catalog.ducklake \
  --data-path=.tmp/duckdb-server/data
```

The mise environment supplies the pinned ADBC driver and isolated DuckDB home.
Connect on `127.0.0.1:5434` in development trust mode:

```sh
psql -h 127.0.0.1 -p 5434 -U postgres
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));
```

## Maintained commands

```sh
mise exec -- just check-fast
mise exec -- just ci
mise exec -- just duckdb-adbc-storage-test
mise exec -- just duckdb-pgwire-workflow-test
mise exec -- just duckdb-spatial-compat-probe
mise exec -- just duckdb-current-benchmark
mise exec -- just duckdb-runtime-static-check
```

Use `just --list` for the complete current command set. Commands are maintained
only when they target registered tests or the DuckDB-only runtime.

## Project focus

Missing behavior follows one decision ladder:

1. DuckDB or official extension functionality;
2. optimizer-visible SQL macro/rewrite;
3. Rust protocol/catalog/control edge; then
4. a vectorized DuckDB extension only for a measured maintained workload.

Rust does not provide row-wise spatial fallback. Shared storage and broad client
compatibility do not block the first single-node release.

Read:

- [docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md) — product focus,
  ownership, extension ladder, and performance direction.
- [ARCHITECTURE.md](./ARCHITECTURE.md) — current boundaries and invariants.
- [ROADMAP.md](./ROADMAP.md) — ordered milestones and measurable exit gates.
- [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) — current evidence and gaps.
- [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) — supported surface and limits.
- [docs/POSTGRESQL_COMPATIBILITY.md](./docs/POSTGRESQL_COMPATIBILITY.md) — target
  catalog/RBAC contract and ordered implementation plan.
- [docs/DUCKDB_ROADMAP_ALIGNMENT.md](./docs/DUCKDB_ROADMAP_ALIGNMENT.md) —
  conditional adoption/deletion gates for upstream DuckDB and DuckLake work.
- [docs/OPERATIONS.md](./docs/OPERATIONS.md) — local runtime and security baseline.
- [docs/QUICKSTART.md](./docs/QUICKSTART.md) — guided setup.
- [DIVERGENCE.md](./DIVERGENCE.md) — retained `arrow-pg` divergence.
- [docs/HISTORY.md](./docs/HISTORY.md) — retired architecture context.

## Development principles

- Prefer upstream DuckDB/DuckLake primitives over QuackGIS mechanisms.
- Validate at protocol/native trust boundaries and fail closed.
- Keep memory proportional to configured bounds, not workload cardinality.
- Require client/workload evidence before adding compatibility or extensions.
- Keep exact DuckDB spatial predicates authoritative under every optimization.
- Delete shims and docs superseded by upstream behavior.

Licensed under the [Apache License 2.0](./LICENSE).
