# Minimal DuckDB-only Kind topology

This directory starts K0 with one QuackGIS StatefulSet, one retained node-local
PV/PVC, generated TLS and password Secrets, readiness/liveness probes, and opt-in
`psql`, psycopg, and GDAL/OGR Jobs. It intentionally contains no service mesh,
PostgreSQL, MinIO, DataFusion, or Sedona services.

Kind provides topology and operational evidence only. Host or constrained-container
profiles remain authoritative for RSS, latency, throughput, spill, and scan-byte
claims. The `hostPath` PV is local-development storage: it survives pod replacement
inside one Kind node but not deletion of the Kind node/container.

## Local Kind with Podman or Docker

Yes: this topology is intended to run locally with rootless Podman. The recipes
auto-select the first usable engine in this order: Podman, then Docker. Set
`CONTAINER_ENGINE=podman` or `CONTAINER_ENGINE=docker` to override that decision.
The same value is passed to Kind as `KIND_EXPERIMENTAL_PROVIDER`.

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

Create a password file without printing it, then build both local images, create
the cluster, load the images into its node, render immutable local digest
references, and wait for the StatefulSet:

```sh
mkdir -p .tmp/kind
printf '%s' 'replace-with-a-local-secret' > .tmp/kind/password
chmod 600 .tmp/kind/password
export QUACKGIS_AUTH_PASSWORD_FILE="$PWD/.tmp/kind/password"
mise exec -- just kind-up-local
mise exec -- just kind-client-gates
```

`kind-up-local` uses the pinned `kindest/node` digest in `cluster.yaml`, builds
`localhost/quackgis-duckdb-runtime:dev` and
`localhost/quackgis-kind-clients:dev`, loads both local tags with
`kind load docker-image`, reads the resulting CRI repository digests from the
Kind node, and deploys only those immutable references. This works for either
Podman or Docker. `imagePullPolicy: IfNotPresent` keeps the node-local images
offline after loading. Cluster access is isolated in `.tmp/kind/kubeconfig`.

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

Secrets are rendered only below ignored `.tmp/kind/`; no key or password belongs
in Git. For a registry-backed run, export pullable image digests and call the
lower-level script directly:

```sh
mkdir -p .tmp/kind
printf '%s' 'replace-with-a-local-secret' > .tmp/kind/password
chmod 600 .tmp/kind/password
export QUACKGIS_AUTH_PASSWORD_FILE="$PWD/.tmp/kind/password"
export QUACKGIS_RUNTIME_IMAGE='registry.example/quackgis@sha256:<64 hex>'
export QUACKGIS_CLIENT_IMAGE='registry.example/quackgis-clients@sha256:<64 hex>'
mise exec -- deploy/kind/up.sh
```

Run each client gate only after the StatefulSet is ready:

```sh
mise exec -- just kind-client-gates
```

The generated certificate is a 30-day self-signed development certificate whose
SAN covers the service DNS names. Client Jobs use `verify-full`; the server uses
TLS-required mode and SCRAM password authentication.
