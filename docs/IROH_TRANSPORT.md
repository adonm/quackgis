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
- `none` remains mandatory. If both endpoints use the `auto` policy, the worker
  selects the evidence-qualified `lz4_block` capability; otherwise the connection
  stays raw.
- Omitted relay configuration selects iroh's public production preset. An
  explicitly empty, malformed, or duplicate custom list is rejected.
- `quackgis-bootstrap` serves only lease requests and keeps a bounded replay
  cache; it never receives or proxies application bytes.
- `quackgis-worker-edge` accepts a bounded number of authenticated connections
  and streams, validates the leased role from the PostgreSQL startup packet,
  handles SSL/GSS negotiation with `N` so TLS is not nested inside iroh, and
  forwards pgwire/cancellation to one loopback complete-worker boundary.
- `quackgis-client` exposes a bounded loopback pgwire listener by default, or a
  mutual-TLS listener when explicitly configured beyond loopback, obtains and
  refreshes one lease, and multiplexes local sessions onto typed worker streams.
  It recognizes only the initial pgwire packet needed to distinguish cancellation
  and does not parse SQL.
- PostgreSQL startup, nested-encryption denial, backend `AuthenticationOk`, all
  access/control messages, and typed cancellation remain raw. Compression framing
  starts only for pgwire application bytes after `AuthenticationOk`.
- The worker requires the loopback pgwire server to begin with PostgreSQL
  `AuthenticationOk`. Any password/SASL challenge is rejected before the local
  client can send credential material across the cluster leg.
- Secret keys are loaded only from bounded, non-symlink files with no group/other
  permissions. `quackgis-keygen` creates a new owner-only file without replacing
  an existing path; `--public-from` derives its public identity without exposing
  the private value.

The K0 package puts the server, worker, bootstrap, and one tiny client in one
ordered StatefulSet Pod. The server remains trust-mode loopback-only, while the
tiny client is the sole Service pgwire endpoint and requires a CA-verified client
certificate. Fixed UDP binds and DNS-resolved direct routes make this package
independent of outbound relay access. Pinned psql, psycopg, and OGR Jobs enter
through that boundary. Denial Jobs prove the Pod address cannot reach the
worker's loopback pgwire port, plaintext is refused, and a client certificate is
required. `just kind-restart-gate` proves ordered reconnect, while
`just kind-secret-rotation-gate` rotates mTLS and edge keys and denies the prior
client certificate. Host profiles remain authoritative for I0 resource budgets.

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
concurrent tunneled sessions. `just iroh-duckdb-relay-smoke` runs that oracle over
a deterministic forced custom relay. The outbound, opt-in
`just iroh-duckdb-public-relay-smoke` runs it through iroh's public preset.

`just iroh-custom-relay-smoke` additionally proves adaptive blocks, typed
cancellation, unusable-direct fallback, same-identity worker restart, credential
rotation, denial of the old credential's next lease, and replacement-client
reconnect. `just iroh-public-relay-smoke` is the smaller outbound reconnect seam.
Neither public-relay command is a required network-dependent CI gate.

## Adaptive block format

Each stream direction owns independent state and retains no dictionary. The
negotiated LZ4 application stream is a sequence of blocks with a 9-byte header:
one raw/LZ4 kind byte, a four-byte decompressed length, a four-byte payload
length, then the payload. Both declared lengths are at most 64 KiB. Raw lengths
must match; compressed payloads must be smaller than their output and may expand
by at most 256:1. Unknown kinds, truncation, corruption, zero/oversized lengths,
length mismatch, and ratio abuse fail before that block reaches pgwire.

Inputs below 1 KiB stay raw. Larger blocks use LZ4 only when they save at least
64 bytes and 12 percent including the fixed selection policy. After two failed
probes, the encoder sends eight blocks raw before sampling again; this bounds CPU
on incompressible streams while allowing a changing stream to enable compression.
There is no state shared across directions, streams, clients, credentials, or
sessions. Idle streams retain no codec buffer.

Payload-free snapshots count application input/wire bytes, saved bytes, blocks,
compressed/small/incompressible/backoff decisions, compression/decompression CPU,
decode failures, and pgwire/cancellation stream counts. The client and worker log
their local snapshot at orderly shutdown; metrics contain no SQL, parameters,
samples, credentials, or paths.

## Committed I0 budgets

`just iroh-transport-profile` compares raw loopback TCP with direct and forced
custom-relay iroh, each with compression `off` and `auto`. It uses the authenticated
bridge and a deterministic trust-mode echo backend to isolate transport cost; the
native DuckDB tests above remain the SQL/COPY correctness oracle. Small,
compressible, xorshift-incompressible, WKB-like, COPY-like, and pgwire-result-like
payloads each produce three connection, first-byte, and throughput samples. The
profile also records process CPU/RSS, cancellation, two concurrent streams, bytes,
codec CPU, ratio, skip reasons, and decode failures in
`.tmp/iroh-transport-profile/`.

The pre-packaging budgets are:

- connection at most 5 s, first byte at most 2 s, and cancellation at most 1 s;
- process RSS growth at most 64 MiB beyond profile idle;
- raw direct iroh p50 throughput at least 5% of loopback TCP and forced relay at
  least 2% (these guard regressions against an unusually fast in-process loopback
  baseline; they are not WAN SLOs);
- automatic incompressible throughput at least 50% of the same-path raw mode,
  with zero compressed incompressible blocks; and
- at least 50% wire-byte savings for the maintained compressible shape, while
  small blocks remain raw and all decode-failure counters remain zero.

Clean smoke/local/reference runs use 8/32/64 MiB per maintained non-small shape:

```sh
just iroh-transport-profile level=smoke bytes=8388608 out=.tmp/iroh-transport-profile/smoke.json
just iroh-transport-profile level=local bytes=33554432 out=.tmp/iroh-transport-profile/local.json
just iroh-transport-profile level=reference bytes=67108864 out=.tmp/iroh-transport-profile/reference.json
```

On source `93c68be`, a 16-logical-CPU AMD Ryzen 7 7700X reference run observed a
17.63 MiB maximum transport RSS delta, 0.38 ms maximum cancellation, 3.41 ms
maximum connection, and 2.99 ms maximum first-byte sample. At 64 MiB, automatic
LZ4 saved 99.57% on the compressible shape; direct/relay compressible p50s were
4715/2380 MiB/s, incompressible p50s were 786/159 MiB/s with zero compressed
blocks, and compression CPU was about 20 ms per direct or relay round trip set.
Raw forced-relay compressible p50 was 147 MiB/s. The release decision is to keep
bounded LZ4 `auto` plus sampling backoff and retain `off`; K0/M5 must rerun these
same budgets against the package and selected hosted relay.

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
  "compression": "auto",
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
  "compression": "auto",
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

`compression` accepts `off` or `auto` independently on worker and client and
defaults to `off`. Both sides must allow automatic negotiation for LZ4 to be
selected. Bootstrap has no compression setting because it never carries
application bytes.

`bind` optionally fixes the local UDP socket used by each iroh endpoint.
`direct_hosts` complements literal `direct_addresses` with bounded DNS
`host:port` routes resolved at startup; this is used for stable Kubernetes Service
names. The local Kind profile sets `disable_relays: true` on all three endpoints
to make the package an outbound-independent direct-path gate. That setting cannot
be combined with `relays`; ordinary operator configuration still treats omitted
relay configuration as the public preset and accepts only explicit non-empty
custom lists.

A tiny-client `listen` address outside loopback fails configuration unless
`local_tls` supplies a server certificate, private key, and client CA:

```json
"local_tls": {
  "certificate_path": "/run/secrets/tls.crt",
  "private_key_path": "/run/secrets/tls.key",
  "client_ca_path": "/run/secrets/client-ca.crt"
}
```

That mode requires the PostgreSQL SSL request and a CA-verified client
certificate before startup or cancellation reaches iroh. TLS terminates at the
local application boundary; it is not nested on the authenticated iroh leg.

The direct TCP backend is an I0 development seam. It stays loopback-only and is
not packaged release ingress. K0 places it behind the same-Pod worker boundary;
the Service exposes only the mutual-TLS tiny client and the direct-denial Job
verifies refusal.

## Security boundary

The credential key is distinct from the ephemeral iroh endpoint key. Bootstrap
signs assignment but never proxies application bytes. A worker must verify the
bootstrap signature, lease lifetime/assignment, current transport endpoint, and
fresh credential proof before accepting a typed pgwire, cancellation, or future
HTTP stream. Neither relay access nor possession of a copied signed lease grants
a database role. Compression cannot begin until that proof succeeds and never
covers lease/control traffic, startup authentication, or cancellation. Application
errors after `AuthenticationOk` are ordinary pgwire application bytes and may be
inside an independent compressed block; cancellation remains a separate raw typed
stream so codec work cannot delay its delivery.
