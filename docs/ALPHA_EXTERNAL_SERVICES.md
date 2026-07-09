# External-service Alpha runbook

This runbook turns the Kind Alpha storage smoke into an evidence plan for real
platform services. It does **not** create a production claim by itself: a claim
exists only after the checklist below is run against the external PostgreSQL
catalog and S3-compatible object store that will hold QuackGIS data.

## Trust boundary

The external-service profile has three separate control planes:

1. QuackGIS pods: stateless pgwire/query workers.
2. PostgreSQL: DuckLake catalog metadata and snapshot head.
3. S3/object store: Parquet data, delete files, and any prewritten pending files.

Catalog and object-prefix backups must be treated as one matched backup set. Do
not claim durability from a PostgreSQL-only restore or an object-prefix-only
restore.

## Required inputs

Use a dedicated catalog database/user and a dedicated object-store prefix for the
run. Never point drills at a shared production prefix unless the drill itself is
the approved production exercise.

| Variable | Purpose | Production expectation |
|---|---|---|
| `EXTERNAL_ALPHA_USE_KIND_EMULATORS=false` | disables the default Kind PostgreSQL/s3s-fs stand-ins | always false for this runbook |
| `EXTERNAL_QUACKGIS_CATALOG_URL` | PostgreSQL DuckLake catalog URL | least-privilege user scoped to the QuackGIS catalog database/schema |
| `EXTERNAL_QUACKGIS_DUCKLAKE_CATALOG_NAME` | DuckLake catalog name | unique per environment/run |
| `EXTERNAL_QUACKGIS_DATA_PATH` | `s3://bucket/prefix` object data path | dedicated prefix with lifecycle policy understood |
| `EXTERNAL_QUACKGIS_S3_ENDPOINT` | S3-compatible API endpoint | HTTPS endpoint unless provider requires empty AWS default |
| `EXTERNAL_QUACKGIS_S3_ACCESS_KEY_ID` / `EXTERNAL_QUACKGIS_S3_SECRET_ACCESS_KEY` | object-store credentials | rotateable secret, not committed or echoed in logs |
| `EXTERNAL_QUACKGIS_S3_REGION` | S3 signing region | provider region |
| `EXTERNAL_QUACKGIS_S3_ALLOW_HTTP=false` | fail closed on cleartext object-store traffic | false outside local emulators |

Example command shape:

```sh
EXTERNAL_ALPHA_USE_KIND_EMULATORS=false \
EXTERNAL_QUACKGIS_CATALOG_URL='postgres://user:pass@postgres.example:5432/quackgis_alpha' \
EXTERNAL_QUACKGIS_DUCKLAKE_CATALOG_NAME='quackgis_alpha_20260709' \
EXTERNAL_QUACKGIS_DATA_PATH='s3://bucket/quackgis-alpha/20260709' \
EXTERNAL_QUACKGIS_S3_ENDPOINT='https://s3.example' \
EXTERNAL_QUACKGIS_S3_ACCESS_KEY_ID='...' \
EXTERNAL_QUACKGIS_S3_SECRET_ACCESS_KEY='...' \
EXTERNAL_QUACKGIS_S3_REGION='us-east-1' \
EXTERNAL_QUACKGIS_S3_ALLOW_HTTP=false \
just kind-external-alpha-smoke
```

## Evidence ladder

Run from least destructive to most failure-oriented. Keep `.tmp/compatibility/`,
`metrics.json`, pod logs, PostgreSQL server logs for the run window, and an object
inventory snapshot before/after mutation or cleanup drills.

| Step | Command / action | Required evidence |
|---|---|---|
| 0. Static preflight | `just probe-static-check` | Kubernetes/probe manifests compile before touching services |
| 1. Emulator preflight | `just kind-external-alpha-smoke` with defaults | local wiring, native delete metadata, and metrics scrape still pass |
| 2. Real-service wiring | `EXTERNAL_ALPHA_USE_KIND_EMULATORS=false ... just kind-external-alpha-smoke` | `storage_ok True`, native delete-file metadata, `metrics_ok True` |
| 3. Multi-pod readers/writers | run `just kind-alpha-smoke` or the component recipes against the external profile equivalent | multi-pod read visibility, writer conflict/retry, QPS, OLAP, and compaction evidence |
| 4. Backup/restore drill | quiesce writers, back up PostgreSQL catalog + object prefix, restore to isolated catalog + prefix, run read-only smoke | restored table discovery, representative counts/bboxes, and spatial query output |
| 5. Credential rotation | rotate PostgreSQL and object-store credentials, roll QuackGIS pods, rerun storage smoke | old pods fail/roll cleanly, new pods pass smoke with no committed data loss |
| 6. Catalog restart | restart/fail over PostgreSQL during a no-write window, then during retry-safe writes | reads recover; failed writes are explicit failures or safe retries, not partial commits |
| 7. Object-store latency/throttling | provider throttle/latency controls or network policy equivalent | probes fail closed or stay within documented budgets; no silent wrong answers |
| 8. Failed-writer cleanup | abort a COPY/DML/compaction job after object writes but before/around metadata commit, then inspect prefix | no partial catalog visibility; suspected orphans are quarantined only after restore point |
| 9. Catalog refresh visibility | vary `QUACKGIS_SHARED_CATALOG_REFRESH_MS` and read from multiple pods after writes | documented staleness behavior matches `docs/OPERATIONS.md` |

Use [MUTATION_FAILURE_DRILLS.md](./MUTATION_FAILURE_DRILLS.md) for the detailed
native DML/compaction fault-injection ladder and acceptance packet.

## Acceptance criteria

An external Alpha evidence packet is credible when it includes:

- command transcript with all secret values redacted;
- QuackGIS image digest and source SHA;
- PostgreSQL/S3 provider names, versions, regions, and relevant service-class
  limits;
- row counts, file counts, object-prefix size, and dataset description;
- `.tmp/compatibility/README.md`, `metrics.json`, and `metrics-dashboard.md`;
- explicit pass/fail notes for each failure drill above;
- backup/restore proof from an isolated restored catalog + object prefix;
- known deviations from the Kind profile.

If any drill is skipped, label the resulting evidence as an **external wiring
smoke**, not as external-service Alpha promotion evidence.

## Rollback and cleanup

- Preserve the catalog backup and object-prefix snapshot until the run is reviewed.
- Delete the temporary QuackGIS deployment first, then the restored/test catalog,
  then the test object prefix.
- Never delete suspected live-prefix orphans directly. Quarantine outside the live
  prefix after a restore point, rerun representative reads, and only then remove
  the quarantined objects under the platform team's retention policy.

## What remains future work

This runbook documents and standardizes the evidence path. QuackGIS still needs
real-service executions of the ladder, automated orphan detection, catalog/object
IO counters, and production RBAC/secret-rotation gates before making production
durability claims.
