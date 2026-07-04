# Changelog

## v0.1.0 — release candidate (Milestones 0–6)

First QuackGIS container release candidate. Thin PostgreSQL facade backed by
DuckDB, sedonadb, and DuckLake.

### Milestone 0 — target reset and docs

- Product target reset from standalone DuckDB extension to Postgres facade
  container.
- All docs consolidated around the new target.
- Reference repo findings documented (pg_duckdb, pg_ducklake, PostDuck,
  DuckFlock).

### Milestone 1 — base image spike

- Multi-stage `container/Dockerfile`: sedonadb built from source (GDAL builder),
  layered on `pg_duckdb` image with spatial runtime libraries.
- Init scripts: DuckDB config, sedonadb load, diagnostic views, PostGIS-compat
  SQL wrappers.
- `docker-compose.yml` for local dev.
- `container/smoke-test.sh` — 6-check black-box psql harness.

### Milestone 2 — DuckLake storage path

- `container/entrypoint.sh` — DuckLake catalog/data-path config from env vars.
- `quackgis.connect_lake()` — idempotent per-session catalog attach.
- `quackgis.create_spatial_table()` — CTAS with layout columns (minx/miny/maxx/
  maxy, quadkey, Hilbert) + partitioning.
- `quackgis.spatial_query_count()` / `exact_query_count()` / `pruning_report()`.
- `container/test-ducklake.sh` — 5-phase persistence + parity test.

### Milestone 3 — SQL/type compatibility shim

- `DOMAIN geometry OVER bytea` with text→geometry cast.
- `&&` (bbox overlap) and `<->` (KNN distance) operators.
- ~50 PostGIS function stubs (constructors, accessors, predicates,
  measurements, set ops, transforms, layout keys).
- Rewrite mode GUC (`strict`/`warn`/`off`).
- `quackgis.rewrite_sql()` — PG-level rewriter delegating to Rust engine.
- `quackgis.compat_check()` — diagnostic function.
- `postgis_version()` / `geometry_columns` / `spatial_ref_sys` stubs.
- `container/test-compat.sh` — 20-check compatibility harness.

### Milestone 4 — black-box facade test suite

- `container/run-all-tests.sh` — unified runner (build, start, all phases).
- `container/tests/postgis-fixtures/` — 6 SQL files (30+ checks).
- `container/tests/test_psycopg.py` — psycopg v3 suite (10 checks).
- `ci/all-checks.sh` restructured into Engine (E1–E6) + Facade (F1–F5) tracks.

### Milestone 5 — Kubernetes hardening

- Helm chart (`deploy/helm/quackgis/`): StatefulSet, Service, PVCs, Secret,
  NetworkPolicy, NOTES.txt.
- Plain K8s manifests (`deploy/k8s/quackgis.yaml`).
- KinD smoke test (`deploy/test-kind.sh`): deploy, query, restart, persistence.
- Security: non-root UID 999, capability drops, NetworkPolicy, OCI labels,
  `.dockerignore`.
- SBOM + image size reporting (`deploy/image-report.sh`).

### Milestone 6 — release candidate

- Compatibility matrix (`docs/COMPATIBILITY_MATRIX.md`): PG 14–18, DuckDB,
  DuckLake modes, object-store backends, client compatibility.
- Operations guide (`docs/OPERATIONS.md`): backup, restore, upgrade, rollback,
  object-store credentials (S3/GCS/Azure), monitoring.
- Known limitations (`docs/KNOWN_LIMITATIONS.md`): consolidated product-behavior
  doc.
- Release script (`deploy/release.sh`): engine checks → build → manifest → SBOM
  → tag guidance.
- Release checklist updated with backup/restore validation, upgrade path
  validation.

### Verification

```
cargo test --lib               → engine unit tests
./tests/run_sql.sh             → SQL regression suite
./ci/all-checks.sh             → full engine CI pipeline
./container/run-all-tests.sh   → full facade test suite
./deploy/test-kind.sh          → Kubernetes smoke test
```

## v0.2.0 — pg_ducklake migration + aggregates + backup + auto-stubs (M7–M10)

### Milestone 7 — switch to pg_ducklake base

- Base image changed from `pgduckdb/pgduckdb:18-main` to
  `pgducklake/pgducklake:18-main`.
- DuckLake tables are now native PG tables via table AM (`USING ducklake`).
  No per-session ATTACH needed — the #1 UX blocker is resolved.
- `.duckdbrc` auto-loads sedonadb on DuckDB instance init.
- Bridge table pattern (`quackgis._bridge`) for standalone spatial calls.
- Init scripts simplified and consolidated.

### Milestone 8 — aggregate function stubs

- `CREATE AGGREGATE` stubs for `st_union_agg`, `st_union`, `st_collect`,
  `st_makeline_agg`, `st_makeline`, `st_envelope_agg`.
- State type `bytea[]`, final function delegates to DuckDB/sedonadb.

### Milestone 9 — pg_dump / backup testing

- `container/test-backup.sh`: pg_dump --insert, DuckLake snapshots, persisted
  spatial query verification.

### Milestone 10 — auto-generated ST_* stubs

- `container/generate-stubs.sh`: reads registry.rs, generates 112 bridge-table
  stubs covering the full 180+ st_* catalog.

### Milestone 11 — CI/CD pipeline

- `.github/workflows/ci.yml`: 5-job pipeline (engine, lint, container-build,
  facade-tests, docs-check) with GHA Docker layer caching.
- `.github/workflows/release.yml`: tag-triggered release (build, push to GHCR,
  SBOM, image report, GitHub Release).
- `docker-bake.hcl`: coordinated multi-target builds with cache scopes.
