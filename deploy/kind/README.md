# Minimal DuckDB-only Kind topology

This directory starts K0 with one QuackGIS StatefulSet, one retained node-local
PV/PVC, generated TLS and password Secrets, readiness/liveness probes, and opt-in
`psql`, psycopg, and GDAL/OGR Jobs. It intentionally contains no service mesh,
PostgreSQL, MinIO, DataFusion, or Sedona services.

Kind provides topology and operational evidence only. Host or constrained-container
profiles remain authoritative for RSS, latency, throughput, spill, and scan-byte
claims. The `hostPath` PV is local-development storage: it survives pod replacement
inside one Kind node but not deletion of the Kind node/container.

## Inputs

Both images must be pullable immutable `image@sha256:<64 hex>` references:

- the runtime image is built from `deploy/Containerfile.duckdb-runtime`;
- the client image must contain `psql`, Python 3 with psycopg 3, and `ogrinfo`, and
  must run as a non-root user.

Secrets are rendered only below ignored `.tmp/kind/`; no key or password belongs
in Git. Create a password file without printing it, export the two image digests,
then start the cluster:

```sh
mkdir -p .tmp/kind
printf '%s' 'replace-with-a-local-secret' > .tmp/kind/password
chmod 600 .tmp/kind/password
export QUACKGIS_AUTH_PASSWORD_FILE="$PWD/.tmp/kind/password"
export QUACKGIS_RUNTIME_IMAGE='registry.example/quackgis@sha256:<64 hex>'
export QUACKGIS_CLIENT_IMAGE='registry.example/quackgis-clients@sha256:<64 hex>'
deploy/kind/up.sh
```

Run each client gate only after the StatefulSet is ready:

```sh
kubectl apply -f .tmp/kind/rendered/clients.yaml
kubectl -n quackgis wait --for=condition=complete job/quackgis-psql --timeout=2m
kubectl -n quackgis wait --for=condition=complete job/quackgis-psycopg --timeout=2m
kubectl -n quackgis wait --for=condition=complete job/quackgis-ogr --timeout=2m
```

The generated certificate is a 30-day self-signed development certificate whose
SAN covers the service DNS names. Client Jobs use `verify-full`; the server uses
TLS-required mode and SCRAM password authentication.
