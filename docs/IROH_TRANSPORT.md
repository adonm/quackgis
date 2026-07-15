# Iroh transport

The active I0 implementation starts in `crates/quackgis-edge`. It is one shared
protocol boundary for the bootstrap, tiny client, and complete worker rather than
three independently evolving wire formats.

## Implemented foundation

- `quackgis/control/1` and `quackgis/edge/1` are fixed ALPN identifiers.
- A bootstrap-signed access lease binds one credential public key, LOGIN role,
  worker endpoint/address, assignment generation, epochs, permitted protocols,
  and a lifetime of at most five minutes.
- Lease refresh proves the registered credential key and binds the request to the
  caller's current iroh transport endpoint and the selected bootstrap.
- Worker authentication adds a fresh challenge. The credential proof covers the
  signed lease, worker challenge, client transport endpoint, and compression
  offers, preventing a copied lease from becoming a bearer credential.
- Control messages are bounded at 16 KiB and reject unknown fields. Roles,
  protocol lists, time ranges, assignments, signatures, proofs, and compression
  negotiation fail closed.
- `none` is the only implemented compression codec and is mandatory. A candidate
  codec will be added only after the I0 direct/relay measurements select one.
- Omitted relay configuration selects iroh's public production preset. An
  explicitly empty, malformed, or duplicate custom list is rejected.
- `quackgis-bootstrap` serves only lease requests and keeps a bounded replay
  cache; it never receives or proxies application bytes.
- `quackgis-worker-edge` accepts a bounded number of authenticated connections
  and streams, validates the leased role from the PostgreSQL startup packet,
  handles SSL/GSS negotiation with `N` so TLS is not nested inside iroh, and
  forwards pgwire/cancellation to one loopback complete-worker boundary.
- `quackgis-client` exposes a bounded loopback pgwire listener, obtains and
  refreshes one lease, and multiplexes local sessions onto typed worker streams.
  It recognizes only the initial pgwire packet needed to distinguish cancellation
  and does not parse SQL.
- The worker requires the loopback pgwire server to begin with PostgreSQL
  `AuthenticationOk`. Any password/SASL challenge is rejected before the local
  client can send credential material across the cluster leg.
- Secret keys are loaded only from bounded, non-symlink files with no group/other
  permissions. `quackgis-keygen` creates a new owner-only file without replacing
  an existing path.

Run the pure protocol evidence with `just iroh-protocol-test`. Run the executable
local-direct seam with `just iroh-direct-smoke`; it creates real bootstrap,
worker, and client iroh endpoints and proves concurrent sessions, local bridge
forwarding, nested-TLS denial, and cancellation against a deterministic fake
trust-mode pgwire backend.

Run `just iroh-duckdb-smoke` for native direct-path parity. One reusable oracle
runs unchanged against direct TCP and the tiny-client bridge and requires equal
result values/types, stable errors, typed parameters, one-row portals, DuckDB
Spatial, commit/rollback/disconnect behavior, successful and malformed COPY
atomicity, cancellation/quarantine, and fresh reconnect state. It also opens two
concurrent tunneled sessions. Public-default and custom relay paths, worker
restart, and CPU/RSS/throughput/compression profiles remain open and are tracked
in [ROADMAP_STATUS.md](./ROADMAP_STATUS.md).

## Operator configuration

Build the four focused binaries:

```sh
cargo build -p quackgis-edge --bins
```

Generate separate bootstrap, worker, credential, and client-transport keys. Each
command prints the corresponding public key and creates the private file with
mode `0600`:

```sh
target/debug/quackgis-keygen --out bootstrap.key
target/debug/quackgis-keygen --out worker.key
target/debug/quackgis-keygen --out credential.key
target/debug/quackgis-keygen --out client-transport.key
```

The worker configuration names the bootstrap public key and a loopback pgwire
server running in trust mode. I0 rejects a non-loopback backend; the backend must
not request a password:

```json
{
  "secret_key_path": "worker.key",
  "bootstrap_public_key": "BOOTSTRAP_PUBLIC_KEY",
  "backend": "127.0.0.1:5434",
  "max_connections": 64,
  "max_streams_per_connection": 64
}
```

Start `quackgis-worker-edge --config worker.json`. Its first stdout line is the
public endpoint document to place in the bootstrap configuration:

```json
{
  "secret_key_path": "bootstrap.key",
  "registered_credential": "CREDENTIAL_PUBLIC_KEY",
  "login_role": "postgres",
  "worker": {
    "endpoint_id": "WORKER_ENDPOINT_ID",
    "direct_addresses": ["192.0.2.10:4242"],
    "relay_url": "https://example-relay.invalid"
  },
  "assignment_generation": 1,
  "lease_ttl_seconds": 60,
  "max_connections": 64
}
```

Start `quackgis-bootstrap --config bootstrap.json` and copy its public endpoint
document into the tiny-client configuration:

```json
{
  "credential_secret_key_path": "credential.key",
  "transport_secret_key_path": "client-transport.key",
  "bootstrap": {
    "endpoint_id": "BOOTSTRAP_ENDPOINT_ID",
    "direct_addresses": ["192.0.2.11:4242"],
    "relay_url": "https://example-relay.invalid"
  },
  "listen": "127.0.0.1:5433",
  "max_connections": 64
}
```

Then run `quackgis-client --config client.json` and point PostgreSQL clients at
`127.0.0.1:5433` with the leased `login_role`. Relative key paths resolve from the
process working directory. Relay configuration is intentionally omitted above,
which selects iroh's public preset. To use hosted relays, add the same explicit,
non-empty form independently to each process:

```json
"relays": ["https://relay.example.com"]
```

The direct TCP backend is an I0 development seam. It must stay loopback-only and
is not release ingress; packaging it behind an owner-protected same-pod/process
boundary and refusing direct application access remain K0/M5 work.

## Security boundary

The credential key is distinct from the ephemeral iroh endpoint key. Bootstrap
signs assignment but never proxies application bytes. A worker must verify the
bootstrap signature, lease lifetime/assignment, current transport endpoint, and
fresh credential proof before accepting a typed pgwire, cancellation, or future
HTTP stream. Neither relay access nor possession of a copied signed lease grants
a database role.
