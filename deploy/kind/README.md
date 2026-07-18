# Minimal DuckDB-only Kind topology

This directory owns the K0 packaged functional boundary. One StatefulSet Pod runs
exactly one complete QuackGIS server, one iroh worker edge, one bootstrap, one
mutual-TLS `postgres` tiny-client bridge, and one separately authenticated
`migration_operator` tiny-client bridge. The complete server binds role-catalog
edge-preauthenticated pgwire only to `127.0.0.1:5434`; it is not a Service
endpoint. Bootstrap maps three distinct proven credentials to exact signed leases:
the existing client credential to `postgres`, a REST service credential to
`authenticator`, and a migration credential to `migration_operator`. Clients never
request a role. The worker requires the startup
user to equal the lease, and the loopback server rejects unknown or `NOLOGIN`
users before `AuthenticationOk`.

A separate Deployment runs two `quackgis-rest` Pods. Each has its own loopback
tiny-client native sidecar, a unique ephemeral transport key, the shared
`authenticator` service credential, and no database password. A namespace-local
ClusterIP balances HTTP over only Ready replicas. The REST pgwire listener is not
published by any Service. A stable internal UDP Service lets those sidecars
reconnect after StatefulSet replacement; the headless Service remains solely for
StatefulSet identity and core startup routing.

The public pgwire tiny-client Service boundary requires mutual TLS. Generated
edge credentials, server/client certificates, retained local PV/PVC,
readiness/liveness probes, and pinned psql, psycopg, and GDAL/OGR Jobs are
included. Three denial Jobs prove that the worker's loopback pgwire port,
plaintext bridge access, and bridge access without a client certificate are
refused. REST gates address each Pod independently, require two ready
EndpointSlice addresses, prove reader/denied OpenAPI and direct-request behavior,
and delete one Pod while the Service remains available. There is no service mesh,
PostgreSQL, MinIO, DataFusion, or Sedona service.

The migration bridge has its own Service, credential, transport key, and client
CA. Its immutable role owns only the maintained migration fixture targets. The
ordinary K0 client certificate is not signed by the migration CA and is denied at
the TLS boundary; the migration certificate is not trusted by the ordinary
pgwire bridge. `kind-postgis-migration-gate` starts a digest-pinned PostGIS native
sidecar only inside an optional Job, runs the non-root migrator from the immutable
runtime image, and moves 10,004 exact scalar/Point/NULL rows through mTLS, iroh,
the common policy edge, ADBC, and DuckLake. The source sidecar and plaintext
loopback source trust are absent from normal K0 startup.

The psycopg 3.2.13 Job is a copied-data gate rather than a scalar connection
smoke. It creates or reuses one client-neutral table, clears it, streams two rows
with PostgreSQL text COPY (including exact WKB and NULLs), closes the connection,
reconnects, and requires exact scalar and `POINT (1 2)` readback. The same gate is
rerun after ordered replacement and mTLS/iroh key rotation. The OGR 3.11.5 Job
waits for that fixture, reads it through the driver's unmodified SQL-result cursor
lifecycle, converts it to GeoJSON, and requires exact `POINT (1 2)`, NULL geometry,
and NULL property values. Its direct layer discovery proves truthful no-FID
behavior. It then asks OGR to append a GeoJSON Point/NULL fixture to a separate
predeclared table with `PG_USE_COPY=YES`; a fresh psycopg connection verifies the
exact committed values. This qualifies OGR reads and predeclared-target COPY, not
OGR table creation or authoritative CRS metadata. The psql Job executes the full
captured `\d+` path.

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
mise exec -- just kind-postgis-migration-gate
mise exec -- just kind-qgis-gate
mise exec -- just kind-restart-gate
mise exec -- just kind-secret-rotation-gate
mise exec -- just kind-rest-jwt-rotation-gate
```

`kind-up-local` builds `localhost/quackgis-duckdb-runtime:dev` and
`localhost/quackgis-kind-clients:dev`, exports provider-native Docker archives,
loads them with `kind load image-archive`, reads the resulting CRI repository
manifest digests from each Kind node, installs matching containerd digest aliases,
and deploys only those immutable references. The runtime image contains the
provenance-pinned DuckDB bundle plus the server, migrator, REST, bootstrap,
worker, client, and keygen binaries. The archive path avoids Kind's `load docker-image` assumption
that a Docker CLI exists when Podman is selected. `imagePullPolicy: IfNotPresent`
keeps node-local images offline after loading. Cluster access is isolated in
`.tmp/kind/kubeconfig`. An existing healthy `quackgis` cluster is reused; an
unreachable named cluster is deleted and recreated before image loading.

The pgwire client gates use `verify-full`, the generated client certificate, and
the leased `postgres` role. The psycopg gate additionally proves copied-data COPY
and reopen behavior, and the OGR gate proves copied-data SQL-result readback,
direct discovery, and OGR-authored COPY through this exact path. The REST
Deployment uses only the separately leased
`authenticator` role; its transaction-local `rest_reader` assumption succeeds and
`rest_denied` sees neither the table path nor direct data. No database password
exists in the packaged edge path: each authenticated bridge requires the
loopback server to return `AuthenticationOk`. Direct pgwire to `5434` is refused
because the complete worker remains loopback-only.

`kind-restart-gate` performs an ordered StatefulSet replacement, requires the
long-lived REST sidecars to discard stale worker sessions and reconnect through
the stable UDP Service, then reruns all pgwire and REST gates.
`kind-secret-rotation-gate` stages replacement mTLS and iroh keys, rolls the
content-hashed core and REST templates, proves the prior client certificate and
prior REST service credential cannot authenticate, then reruns the current
copied-data, denial, replica, and failover gates. `kind-rest-jwt-rotation-gate`
replaces the shared JWT key, rolls only the REST Deployment, accepts new-key
tokens, and denies an old-key token against each replacement Pod. This is a
bounded replacement operation after rollout, not a zero-downtime multi-key
overlap contract. Failed rotation keeps previous owner-only material under
`.tmp/kind/` for explicit recovery rather than silently deleting it.

`kind-qgis-gate` runs the normal client/REST matrix first, then starts the
separately pinned QGIS 3.44.11 image. Its read-only query layer proves exact
fields, Point/NULL values, expression and subset filters, extent, a spatial
viewport identify request, and non-empty offscreen rendering. The QGIS image is
not part of normal topology startup.

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
