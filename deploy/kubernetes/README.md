# Kubernetes deployment example

This directory is a production-style example for the Alpha PostgreSQL/S3 storage
profile. It is not a Helm chart and is not a production claim; use it as a
reviewable starting point for platform manifests after the Alpha gates pass in
your environment.

The example assumes secrets are created outside Git:

```sh
kubectl -n quackgis create secret generic quackgis-storage \
  --from-literal=catalog-url='postgres://USER:PASSWORD@postgres.example:5432/quackgis' \
  --from-literal=ducklake-catalog-name='quackgis' \
  --from-literal=data-path='s3://bucket/quackgis' \
  --from-literal=s3-endpoint='https://s3.example' \
  --from-literal=s3-access-key-id='...' \
  --from-literal=s3-secret-access-key='...' \
  --from-literal=s3-region='us-east-1'

kubectl -n quackgis create secret generic quackgis-auth \
  --from-literal=readwrite-user='quackgis_rw' \
  --from-literal=readwrite-password='...' \
  --from-literal=readonly-user='quackgis_ro' \
  --from-literal=readonly-password='...'

kubectl -n quackgis create secret tls quackgis-tls \
  --cert=tls.crt \
  --key=tls.key
```

Then review and apply:

```sh
just probe-static-check
kubectl apply -f deploy/kubernetes/quackgis-alpha.yaml
```

Important defaults in the example:

- password/SCRAM mode is enabled;
- pgwire TLS is enabled from `quackgis-tls`;
- metrics are exposed on an internal Service port (`9187`);
- CPU/memory requests and limits are explicit;
- pods run non-root with a read-only root filesystem and an `emptyDir` `/tmp`;
- storage credentials are secret references, never committed literal values.

Validate with the same evidence path as Kind where possible: storage smoke,
restore drill, QPS/OLAP scan budgets, writer conflict/retry, and client probes.
