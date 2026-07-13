# QuackGIS REST interface

`quackgis-rest` is a separate stateless HTTP process that connects to QuackGIS
through pgwire. It extends the URL parser, request AST, parameterized SQL builder,
and schema model from `joshburgess/pg-rest-server` at immutable revision
`b7915d3c3361f0fee45de6e292e62f6f6186375f`. Keeping the process separate makes
HTTP replicas independently load-balanceable and ensures its compatibility tests
also exercise QuackGIS pgwire behavior.

This is an intentionally read-only first slice. It is not yet a claim of complete
PostgREST compatibility or of upstream's 1,013-case PostgreSQL result.

The durable target is not a REST-specific security/catalog implementation.
QuackGIS will own PostgreSQL 18 roles, privileges, session identity, catalog
projection, and bounded request context; REST will exercise those capabilities as
an ordinary pgwire client. See
[POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md) for the ordered plan.

## Trust boundaries

- `/live` and `/ready` are unauthenticated operational endpoints.
- Every API, OpenAPI, and reload request requires
  `Authorization: Bearer <token>`.
- The token is loaded from a file, must contain at least 32 non-whitespace bytes,
  and is compared in constant time. It is never accepted on the command line.
- The pgwire URL carries the database identity. Use a QuackGIS read-only identity;
  the REST process does not emulate PostgreSQL role switching or RLS.
- The HTTP listener defaults to `127.0.0.1`. Terminate public TLS and enforce
  request/rate limits at the load balancer before binding it more broadly.
- A CA file forces TLS and enables hostname-verified rustls for the pgwire
  connection. A URL that requires TLS is rejected if the CA is absent. Omitting
  both selects plaintext and is suitable only for a same-host/private-loopback
  development connection.
- `QUACKGIS_REST_TABLES` is a required explicit comma-separated table allowlist.
  Startup and reload fail if an entry is absent from DuckDB's `main` schema. User
  filter values remain text bind parameters; table and column names must resolve
  through this bounded in-memory schema cache. The pgwire user's read policy is a
  second independent authorization boundary.
- Query execution has a fail-closed timeout. Native errors are bounded before
  entering the HTTP response.

## Current PostgREST subset

Supported now:

- `GET /<table>` and `HEAD /<table>`;
- `select`, scalar filters, grouped `and`/`or`, `order`, `limit`, and `offset`
  using the pinned upstream parser/query engine;
- JSON array responses generated in DuckDB;
- authenticated OpenAPI 3 discovery at `/`;
- explicit authenticated schema refresh at `POST /reload`; and
- `/live` and database-backed `/ready`.

QuackGIS-specific coverage includes maintained WKB columns, which are emitted as
DuckDB's escaped binary JSON string. GeoJSON projection, geometry/SRID metadata,
relationships/embedding, count preferences, CSV, singular media types, RPC, JWT
roles, and all mutations remain open. Unsupported HTTP methods fail closed with
`405`; missing schema-cache resources return `404`.

## Target role-aware architecture

The current bearer token and per-process `information_schema.columns` cache are
bootstrap controls. The target request path is:

```text
JWT request
    │ validate signature/issuer/audience/time/role mapping
    ▼
quackgis-rest replica
    │ BEGIN
    │ SET LOCAL ROLE <configured API role>
    │ set_config('request.jwt.claims', <bound JSON>, true)
    │ catalog or application query
    │ COMMIT
    ▼
QuackGIS pgwire catalog/session/authorization boundary
    ▼
DuckDB + official DuckLake
```

Delivery is intentionally staged as an implementation decomposition of the
M3/M5 gates in [../ROADMAP.md](../ROADMAP.md):

1. replace direct DuckDB schema assumptions with maintained PostgreSQL catalog
   discovery while retaining the explicit REST exposure ceiling;
2. add JWT verification, one authenticator identity, bounded role mapping,
   transaction-local role/context, and role-aware OpenAPI cached by role and
   schema/security epoch;
3. package multiple immutable stateless replicas and prove denial, readiness,
   load balancing, cache invalidation, and credential rotation; then
4. add relationships and mutations only after common key metadata, object
   privileges, cancellation/transaction outcomes, and maintained bbox invariants
   pass through direct pgwire.

Table and operation RBAC is sufficient for role-specific paths and methods. It is
not RLS. RLS-protected reads or writes remain blocked until QuackGIS has a
structural policy model, matching `pg_policy` behavior, and an adversarial bypass
suite across every maintained read/write shape.

The role-specific OpenAPI cache key is:

```text
effective role + catalog/security epoch + REST exposure configuration
```

An operation omitted by OpenAPI must also be denied when requested directly. An
operation allowed by database grants can still be hidden by the REST exposure
ceiling, but REST can never widen a database grant.

## Run locally

Start QuackGIS first, create a token without printing it, and use a read-only
pgwire identity:

```sh
mkdir -p .tmp/rest
python3 - <<'PY'
from pathlib import Path
import secrets
Path('.tmp/rest/token').write_text(secrets.token_urlsafe(32), encoding='utf-8')
PY
chmod 600 .tmp/rest/token

export QUACKGIS_REST_DATABASE_URL='postgres://reader:password@127.0.0.1:5434/quackgis'
export QUACKGIS_REST_BEARER_TOKEN_FILE="$PWD/.tmp/rest/token"
export QUACKGIS_REST_TABLES='points,roads'
mise exec -- just rest-server
```

For TLS pgwire, add:

```sh
export QUACKGIS_REST_DATABASE_CA="$PWD/path/to/ca.crt"
```

Then query with the token loaded without echoing it:

```sh
curl --fail-with-body \
  -H "Authorization: Bearer $(cat .tmp/rest/token)" \
  'http://127.0.0.1:3000/points?select=id,name&id=gte.2&order=id.desc&limit=10'
```

## Compatibility gates

```sh
mise exec -- just rest-check
mise exec -- just rest-postgrest-smoke
```

The native smoke starts an actual DuckDB/DuckLake pgwire server and REST router,
then proves authentication denial, OpenAPI discovery, table discovery, projection,
typed filtering, ordering, pagination, missing-resource behavior, mutation denial,
and escaped WKB transport. These cases seed the QuackGIS extension of the
PostgREST contract. Each additional PostgREST behavior must enter this executable
suite before being listed as supported.

Upstream's differential runner still requires a real PostgreSQL fixture and
PostgREST reference process. It remains useful for parser/HTTP parity, but its
PostgreSQL roles, RLS, catalogs, LISTEN/NOTIFY, functions, and JSON SQL cannot be
claimed for DuckDB without explicit QuackGIS implementations and tests.

As common catalog/RBAC work lands, applicable upstream schema-cache, role, and
HTTP cases should move into two gates: a normalized PostgreSQL/PostgREST reference
comparison and an actual QuackGIS-pgwire case. Passing parser-only cases does not
establish database semantics.
