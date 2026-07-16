# QuackGIS REST interface

`quackgis-rest` is a separate stateless HTTP process that connects to QuackGIS
through pgwire. It extends the URL parser, request AST, parameterized SQL builder,
and schema model from `joshburgess/pg-rest-server` at immutable revision
`b7915d3c3361f0fee45de6e292e62f6f6186375f`. Keeping the process separate makes
HTTP replicas independently load-balanceable and ensures its compatibility tests
also exercise QuackGIS pgwire behavior.

This is an intentionally read-only, role-aware slice. It is not yet a claim of
complete PostgREST compatibility or of upstream's 1,013-case PostgreSQL result.

The durable target is not a REST-specific security/catalog implementation.
QuackGIS will own PostgreSQL 18 roles, privileges, session identity, catalog
projection, and bounded request context. Local 1.0 exercises those capabilities
through this ordinary pgwire sidecar connected to the packaged tiny client, not a
direct worker listener. Shared 1.x carries HTTP as a typed stream on the same tiny
client edge connection to the assigned complete worker, reusing the same access
lease, policy, admission, epochs, and audit identity. See
[POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md) for the ordered plan.

## Trust boundaries

- `/live` and `/ready` are unauthenticated operational endpoints.
- Every API, OpenAPI, and reload request requires a signed JWT in
  `Authorization: Bearer <token>`.
- The current profile accepts HS256 only. Its operator-provisioned secret is read
  from a bounded regular file, must contain 32–4096 non-whitespace bytes, and is
  never accepted on the command line. The file is re-read for every verification
  and readiness check so an atomic replacement rotates the key without a process
  restart. Missing/malformed material makes `/ready` fail and all API verification
  fail closed. Signature, exact issuer/audience, expiry, optional not-before,
  token/claim size, and the configured `role` allowlist are validated before
  database work. Invalid tokens and unavailable key material receive the same
  generic API error.
- The pgwire URL carries one privileged authenticator identity but must not carry
  its password. The password is read from a bounded, owner-only, non-symlink
  regular file. REST validates that file before each database operation and uses
  it only while opening a connection. A file revision change discards the prior
  connection and reconnects with the replacement credential; invalid or
  database-mismatched material makes `/ready` and database work fail closed.
  Every discovery and read transaction assumes only the validated, statically
  configured role and binds normalized claims to transaction-local
  `request.jwt.claims`. Database grants remain authoritative; claims do not
  implement RLS.
- The HTTP listener defaults to `127.0.0.1`. Terminate public TLS and enforce
  request/rate limits at the load balancer before binding it more broadly.
- A CA file forces TLS and enables hostname-verified rustls for the pgwire
  connection. A URL that requires TLS is rejected if the CA is absent. Omitting
  both selects plaintext and is suitable only for a same-host/private-loopback
  development connection.
- `QUACKGIS_REST_TABLES` is a required explicit comma-separated table allowlist.
  Startup fails if an entry is absent from every configured JWT role's filtered
  PostgreSQL `public` schema. A role may see a strict subset. User filter values
  and claims remain bind parameters; table and column names must resolve through
  that role's bounded in-memory schema cache. The authenticator's legacy read
  policy and effective role's grants are independent authorization ceilings.
- Query execution has a fail-closed timeout. Native errors are bounded before
  entering the HTTP response.
- Before OpenAPI or request SQL is generated, the sidecar re-reads the maintained
  role-filtered catalog and compares a length-framed SHA-256 revision over only
  REST-exposed columns, types, nullability, and defaults. An unchanged revision
  reuses the immutable cache; a changed revision replaces it. Validation failure
  returns `503`. Database authorization remains authoritative if state changes
  between validation and execution.

## Current PostgREST subset

Supported now:

- `GET /<table>` and `HEAD /<table>`;
- `select`, scalar filters, grouped `and`/`or`, `order`, `limit`, and `offset`
  using the pinned upstream parser/query engine;
- JSON array responses generated in DuckDB;
- authenticated role-aware OpenAPI 3 discovery at `/`;
- automatic role-filtered schema revision validation before API/OpenAPI requests;
- explicit authenticated schema validation at `POST /reload`; and
- `/live` and database-backed `/ready`.

QuackGIS-specific coverage includes maintained WKB columns, which are emitted as
DuckDB's escaped binary JSON string. GeoJSON projection, geometry/SRID metadata,
relationships/embedding, count preferences, CSV, singular media types, RPC, and
all mutations remain open. Unsupported HTTP methods fail closed with `405`;
resources absent from the JWT role's cache return `404`.

## Target role-aware architecture

The implemented request path is:

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

1. catalog-backed PostgreSQL discovery plus the explicit REST exposure ceiling —
   complete;
2. JWT verification, one authenticator identity, bounded role mapping,
   transaction-local role/context, and role-aware OpenAPI — complete at the
   direct-pgwire preview boundary;
3. automatic role-filtered schema/security revision invalidation — complete at
   the direct-pgwire boundary; next consume shared monotonic epochs and package
   multiple immutable stateless replicas with denial, readiness, load balancing,
   and credential rotation; then
4. add relationships and mutations only after common key metadata, object
   privileges, cancellation/transaction outcomes, and maintained bbox invariants
   pass through direct pgwire.

Table and operation RBAC is sufficient for role-specific paths and methods. It is
not RLS. RLS-protected reads or writes remain blocked until QuackGIS has a
structural policy model, matching `pg_policy` behavior, and an adversarial bypass
suite across every maintained read/write shape.

The current role-specific cache key is `effective role + REST exposure
configuration + role-filtered catalog revision`. Every authenticated API/OpenAPI
request validates that revision in a transaction under the effective role, so a
schema or visibility change replaces the immutable cache before SQL generation.
`POST /reload` remains an explicit validation endpoint, not a correctness
requirement. Local 1.0 should replace per-request catalog reads with the shared
monotonic schema/security epochs once that packaged control contract exists.

An operation omitted by OpenAPI must also be denied when requested directly. An
operation allowed by database grants can still be hidden by the REST exposure
ceiling, but REST can never widen a database grant.

## Run locally

Start QuackGIS with an immutable role graph containing LOGIN role
`authenticator`, an assumable `api_reader`, and the intended grants. Then create
an HS256 key without printing it and configure the REST process:

```sh
mkdir -p .tmp/rest
python3 - <<'PY'
from pathlib import Path
import secrets
Path('.tmp/rest/jwt-secret').write_text(secrets.token_urlsafe(48), encoding='utf-8')
Path('.tmp/rest/database-password').write_text(secrets.token_urlsafe(32), encoding='utf-8')
PY
chmod 600 .tmp/rest/jwt-secret .tmp/rest/database-password

export QUACKGIS_REST_DATABASE_URL='postgres://authenticator@127.0.0.1:5434/quackgis'
export QUACKGIS_REST_DATABASE_PASSWORD_FILE="$PWD/.tmp/rest/database-password"
export QUACKGIS_REST_JWT_SECRET_FILE="$PWD/.tmp/rest/jwt-secret"
export QUACKGIS_REST_JWT_ISSUER='https://issuer.example'
export QUACKGIS_REST_JWT_AUDIENCE='quackgis-rest'
export QUACKGIS_REST_JWT_ROLES='api_reader'
export QUACKGIS_REST_TABLES='points,roads'
mise exec -- just rest-server
```

For TLS pgwire, add:

```sh
export QUACKGIS_REST_DATABASE_CA="$PWD/path/to/ca.crt"
```

Create a five-minute test token and query without exposing the signing key:

```sh
export QUACKGIS_REST_TOKEN="$(python3 - <<'PY'
import base64, hashlib, hmac, json, os, time
enc = lambda value: base64.urlsafe_b64encode(value).rstrip(b'=').decode()
header = enc(json.dumps({'alg': 'HS256', 'typ': 'JWT'}, separators=(',', ':')).encode())
claims = enc(json.dumps({
    'iss': os.environ['QUACKGIS_REST_JWT_ISSUER'],
    'aud': os.environ['QUACKGIS_REST_JWT_AUDIENCE'],
    'sub': 'local-test', 'role': 'api_reader', 'exp': int(time.time()) + 300,
}, separators=(',', ':')).encode())
body = f'{header}.{claims}'
key = open(os.environ['QUACKGIS_REST_JWT_SECRET_FILE'], 'rb').read().strip()
print(f'{body}.{enc(hmac.new(key, body.encode(), hashlib.sha256).digest())}')
PY
)"
curl --fail-with-body \
  -H "Authorization: Bearer $QUACKGIS_REST_TOKEN" \
  'http://127.0.0.1:3000/points?select=id,name&id=gte.2&order=id.desc&limit=10'
unset QUACKGIS_REST_TOKEN
```

Rotate the signing key by staging a valid owner-protected file in the same
directory and atomically replacing the configured path. Issue tokens under the
new key only after `/ready` returns `200`; previously signed tokens fail
immediately after replacement. There is deliberately no old/new overlap window:

```sh
python3 - <<'PY'
from pathlib import Path
import os, secrets
target = Path(os.environ['QUACKGIS_REST_JWT_SECRET_FILE'])
staged = target.with_name(target.name + '.next')
staged.write_text(secrets.token_urlsafe(48), encoding='utf-8')
os.chmod(staged, 0o600)
os.replace(staged, target)
PY
curl --fail-with-body http://127.0.0.1:3000/ready
```

Rotate the authenticator credential as an ordered database restart operation.
Stage and atomically replace `QUACKGIS_REST_DATABASE_PASSWORD_FILE`; readiness
must fail while the file and database disagree. Restart QuackGIS against the same
DuckLake state with the matching authenticator password. REST reopens pgwire with
the new credential without restarting, readiness and committed reads recover,
and the old password must be denied. There is no old/new password overlap in the
REST process.

## Compatibility gates

```sh
mise exec -- just rest-check
mise exec -- just rest-postgrest-smoke
```

The native smoke starts an actual DuckDB/DuckLake pgwire server and REST router,
then proves HS256 validation/denial, one SCRAM authenticator, transaction-local
role/claim cleanup, grant-backed PostgreSQL catalog discovery, role-specific
OpenAPI/direct denial, database denial even with an intentionally stale/wide
cache, automatic repair of that stale role cache, and live-column revision
invalidation, plus atomic JWT key replacement, new-key acceptance, old-key
denial, invalid-key readiness failure, owner-only database-password validation,
credential-mismatch readiness failure, same-state database restart, automatic
new-password reconnect without a REST restart, old-password denial, projection,
typed filtering, ordering, pagination, missing-resource behavior, mutation
denial, and escaped WKB transport. These cases seed the QuackGIS extension of the
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
