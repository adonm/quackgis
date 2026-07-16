# Minimal DuckDB-only Kind topology

This directory owns the K0 packaged functional boundary. One StatefulSet Pod runs
exactly one complete QuackGIS server, one iroh worker edge, one bootstrap, and one
tiny-client bridge. The complete server binds trust-mode pgwire only to
`127.0.0.1:5434`; it is not a Service endpoint. The worker carries that loopback
session over authenticated iroh, and the only application pgwire port is the tiny
client on `5432`. The application Service publishes only Ready Pods; a separate
headless Service may publish unready addresses solely for bootstrap/worker UDP
startup routing.

The tiny-client Service boundary requires mutual TLS. Generated edge credentials,
server/client certificates, retained local PV/PVC, readiness/liveness probes, and
pinned psql, psycopg, and GDAL/OGR Jobs are included. Three denial Jobs prove that
the worker's loopback pgwire port, plaintext bridge access, and bridge access
without a client certificate are refused. There is no service mesh, PostgreSQL,
MinIO, DataFusion, or Sedona service.

The complete server, worker, and bootstrap use Kubernetes native sidecar ordering;
the tiny client is the regular container. Shutdown therefore removes ingress
first, then bootstrap and worker transport, then the DuckDB server. Every edge
binary handles both SIGINT and Kubernetes SIGTERM. Kind provides topology and
operational evidence only. Host or constrained-container profiles remain
authoritative for RSS, latency, throughput, spill, and scan-byte claims. The
`hostPath` PV is local-development storage: it survives Pod replacement inside one
Kind node but not deletion of the Kind node/container.

## Local Kind with Podman or Docker

This topology is intended to run locally with rootless Podman and also supports
Docker. The recipes auto-select the first usable engine in this order: Podman,
then Docker. Set `CONTAINER_ENGINE=podman` or `CONTAINER_ENGINE=docker` to override
that decision. The same value is passed to Kind as `KIND_EXPERIMENTAL_PROVIDER`.

Install the repository-pinned Kind 0.32.0 and kubectl 1.36.2 tools, then inspect
all core, Kind, container, and optional named-client dependencies:

```sh
mise install
mise exec -- just doctor
mise exec -- just doctor-kind
```

Rootless Podman requires cgroup v2 and user cgroup delegation. `just doctor`
prints the detected Podman rootless/cgroup/runtime state. The upstream
[Kind rootless guide](https://kind.sigs.k8s.io/docs/user/rootless/) also documents
subuid/subgid, PID-limit, inotify, networking-module, and `Delegate=yes`
troubleshooting. The minimal QuackGIS topology does not bind privileged host
ports.

Build both local images, create the edge keys and development CA/certificates
under ignored `.tmp/kind/`, create the cluster, load the images by digest, and
wait for the complete packaged path:

```sh
mise exec -- just kind-up-local
mise exec -- just kind-client-gates
mise exec -- just kind-restart-gate
mise exec -- just kind-secret-rotation-gate
```

`kind-up-local` builds `localhost/quackgis-duckdb-runtime:dev` and
`localhost/quackgis-kind-clients:dev`, exports provider-native Docker archives,
loads them with `kind load image-archive`, reads the resulting CRI repository
manifest digests from each Kind node, installs matching containerd digest aliases,
and deploys only those immutable references. The runtime image contains the
provenance-pinned DuckDB bundle plus the server, bootstrap, worker, client, and
keygen binaries. The archive path avoids Kind's `load docker-image` assumption
that a Docker CLI exists when Podman is selected. `imagePullPolicy: IfNotPresent`
keeps node-local images offline after loading. Cluster access is isolated in
`.tmp/kind/kubeconfig`. An existing healthy `quackgis` cluster is reused; an
unreachable named cluster is deleted and recreated before image loading.

The client gates use `verify-full`, the generated client certificate, and the
leased `postgres` role. No database password crosses the cluster leg: the worker
requires the loopback server to return `AuthenticationOk`, and the bridge's mTLS
boundary authenticates the packaged clients independently. Direct pgwire to
`5434` is refused because the complete worker remains loopback-only.

`kind-restart-gate` performs an ordered StatefulSet replacement and reruns all six
positive/negative Jobs. `kind-secret-rotation-gate` stages replacement mTLS and
iroh keys, rolls the content-hashed Pod template, proves the prior client
certificate is denied, then reruns the current client Jobs. Failed rotation keeps
the previous owner-only material under `.tmp/kind/` for explicit recovery rather
than silently deleting it.

Delete the disposable node when finished. This deletes its node-local PV data by
design even though the Kubernetes PV reclaim policy is `Retain`:

```sh
mise exec -- just kind-down
```

If rootless Podman reports missing cgroup delegation despite a correctly
configured host, invoke the recipe in a delegated user scope:

```sh
systemd-run --scope --user -p Delegate=yes \
  mise exec -- just kind-up-local
```

## Remote immutable-image inputs

Both images must be pullable immutable `image@sha256:<64 hex>` references:

- the runtime image is built from `deploy/Containerfile.duckdb-runtime`;
- the client image must contain `psql`, Python 3 with psycopg 3, and `ogrinfo`, and
  must run as a non-root user.

The renderer also needs a packaged `quackgis-keygen` binary. Set
`QUACKGIS_KEYGEN=/path/to/quackgis-keygen` when it is not at
`target/release/quackgis-keygen`. Secrets are rendered only below ignored
`.tmp/kind/`; no key or certificate belongs in Git. For a registry-backed run:

```sh
export QUACKGIS_KEYGEN=/path/to/quackgis-keygen
export QUACKGIS_RUNTIME_IMAGE='registry.example/quackgis@sha256:<64 hex>'
export QUACKGIS_CLIENT_IMAGE='registry.example/quackgis-clients@sha256:<64 hex>'
mise exec -- deploy/kind/up.sh
mise exec -- just kind-client-gates
```

The generated certificates are signed by a 30-day local development CA. The
server certificate SAN covers the Service DNS names; the independent client
certificate has client-auth usage. The generated private edge files are copied by
a root init container to mode `0600` before non-root processes load them.

The committed local topology disables relays and uses fixed direct UDP ports so
it is deterministic and does not depend on outbound access. The packaged binaries
retain the normal operator contract: omit relay configuration for iroh's public
preset or replace `disable_relays` with the same explicit non-empty `relays` list
in bootstrap, worker, and client configuration. Hosted-relay credentials must be
supplied through a protected Secret and must not be committed or logged.
