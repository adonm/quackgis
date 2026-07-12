# Release evidence policy

Every release claim must be reviewable from artifacts tied to one source SHA and
one exact DuckDB/extension bundle.

## Current preview evidence

| Artifact/gate | Purpose |
|---|---|
| `just check-fast` | Rust formatting, clippy, unit and compile-time integration targets |
| `just duckdb-adbc-storage-test` | native Arrow/DuckLake write/query/transaction/reopen behavior |
| `just duckdb-pgwire-workflow-test` | real server protocol, auth, policy, COPY, transaction, spatial, restart behavior |
| `just duckdb-spatial-compat-probe` | maintained spatial disposition/result classification |
| `just duckdb-runtime-static-check` | immutable load-only runtime-image contract |
| `.tmp/duckdb/manifest.json` | native library/extension paths and SHA-256 values |
| `.tmp/duckdb-current-benchmark/manifest.json` | deterministic direct DuckDB/ADBC/pgwire correctness and liveness comparison |

`just ci` is the required aggregate local gate after native bootstrap. There is no
active hosted workflow; a future CI workflow must call the same Justfile recipes
and may upload verification manifests, not redistributable native binaries.

## Evidence levels

| Level | Use |
|---|---|
| smoke | fast code/contract regression; never a scale claim |
| local | complete functional scenario and oracle at reduced scale |
| reference | exact scale/duration/budgets on a named host; may close local gates |
| external | published-artifact or managed-service proof explicitly required by a gate |

Performance evidence runs directly on the named host or one constrained
container. Minimal Kind runs provide packaged topology, client, TLS, lifecycle,
recovery, upgrade, mixed-workload, and soak evidence; they are not primary
performance evidence. PostgreSQL/MinIO in Kind is M6 rehearsal only.

## Local 1.0 release packet

The release packet must include:

- source SHA, version, Rust lockfile, DuckDB/extension versions and digests;
- server binary checksum and runtime image digest;
- all M1–M4 test/client/performance reports;
- named client versions and copied-data manifests;
- exact 10M profile hardware/data/budget/results;
- query/COPY memory, spill, cancellation, and admission evidence;
- backup/restore and upgrade/rollback transcripts;
- TLS/auth/secret-rotation evidence;
- 24-hour soak summary and raw metrics/log locations; and
- known limits copied from `COMPATIBILITY.md` and `ROADMAP_STATUS.md`.

## Evidence rules

- Artifacts identify exact source SHA and native bundle.
- Artifacts identify their evidence level and execution environment.
- Performance claims identify rows, bytes, files, row groups, load method,
  hardware, concurrency, plans, and budget outcome.
- Result checks use counts/checksums/exact spatial oracles, not successful exit
  alone.
- Metrics and logs exclude SQL text, parameters, credentials, WKB, signed URIs,
  and sensitive paths.
- Missing required evidence downgrades the artifact to a preview/manual build.
- Retired-engine, emulator-only, static-profile, and design evidence cannot satisfy
  current release gates.

## Shared 1.x evidence

Shared claims additionally require two repeatable managed-service runs covering
multi-process visibility, conflicts/response loss, throttling/interruption,
credential rotation, backup/restore, cleanup, independent DuckDB reopen, and a
24-hour measured topology. Local emulator results are companion evidence, not
managed-service proof.
