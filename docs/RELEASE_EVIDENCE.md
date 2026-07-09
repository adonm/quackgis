# Release evidence policy

Every release should be reviewable without rerunning probes. The release artifact
bundle ties a source SHA to binaries, container images, compatibility probes,
storage probes, metrics dashboards, and known limits.

## Release artifact set

| Artifact | Producer | Purpose |
|---|---|---|
| `quackgis-server-<version>-linux-x86_64.tar.gz` | `CI artifacts` workflow | release binary |
| `*.sha256` | `CI artifacts` workflow | binary integrity |
| GHCR image tags | `CI artifacts` workflow | runtime image identity |
| `release-evidence-<version>.json` | `CI artifacts` workflow | machine-readable index for release evidence |
| `compatibility-report-<sha>-<run_id>` | `Compatibility probes` workflow | QGIS/OGR/GeoServer/OSM logs and rendered report |
| `compatibility-metrics-<sha>-<run_id>` | `Compatibility probes` workflow | compatibility `metrics.json` |
| `storage-kind-logs-<recipe>-<sha>-<run_id>` | `Storage smoke` workflow | storage probe logs/report |
| `storage-metrics-<recipe>-<sha>-<run_id>` | `Storage smoke` workflow | storage `metrics.json` |
| `benchmark-report-<recipe>-<sha>-<run_id>` | `Benchmark ladder` workflow | manual LayoutBench/QPS/OLAP benchmark report |
| `benchmark-metrics-<recipe>-<sha>-<run_id>` | `Benchmark ladder` workflow | manual benchmark `metrics.json` |
| `postgis-regress-<sha>-<run_id>` | `PostGIS regress subset` workflow | PostGIS subset log, metrics, and dashboard |
| `metrics-dashboard.md` | scheduled/manual probe workflows | compact human review surface next to `metrics.json` |

The dashboard is a summary, not the source of truth. Keep the original
`metrics.json`, probe logs, and rendered compatibility/storage report in the
release evidence packet.

## Minimum release gate

Before publishing a public tag, attach or reference evidence for the same source
SHA:

1. `just check-fast` via CI or a local transcript;
2. compatibility probe artifact from the maintained workflow;
3. storage smoke artifact, at least `kind-alpha-smoke`;
4. budget check transcript, for example
   `just metrics-budget-check path=<artifact-dir> require_budgeted=true` on the
   selected compatibility/storage/benchmark metrics;
5. PostGIS regress subset artifact;
6. PostGIS fixture summary from `just postgis-conformance-summary` when the
   conformance ledger changed;
7. release binary checksum and image digest/tags;
8. known limitations copied from `docs/COMPATIBILITY.md`,
   `docs/POSTGIS_CONFORMANCE.md`, and roadmap open items.

If any scheduled artifact is missing for the release SHA, label the release as a
preview/manual build and include the exact replacement command transcript.

## Evidence review checklist

- Source SHA in all artifacts matches the release manifest.
- Compatibility/storage/benchmark workflows ran `metrics-budget-check` on their
  uploaded `metrics.json`, including required-check assertions for expected
  recipe outputs; reports do not contain `❌ fail` rows, and the gate did not run
  against an empty artifact directory.
- `metrics-dashboard.md` is present beside `metrics.json` when workflows produced
  metrics.
- `just metrics-budget-check require_budgeted=true` passes for release-selected
  metrics artifacts.
- QPS/OLAP scan budgets did not regress unexpectedly.
- Native DML/compaction/abort counters and row-count checks match the roadmap
  claim.
- PostGIS regress pass-rate is recorded and unsupported surfaces are documented in
  `docs/POSTGIS_CONFORMANCE.md`.
- External PostgreSQL/S3 claims are not made unless the evidence packet includes
  `docs/ALPHA_EXTERNAL_SERVICES.md` drill results.
- Release notes call out any DuckLake alignment or reference-reader caveat from
  `docs/DUCKLAKE_ALIGNMENT.md`.

## Manifest fields

`release-evidence-<version>.json` points to artifact naming patterns instead of
embedding large probe logs. Its `scheduled_evidence` object names metrics/log
artifact prefixes and canonical filenames:

- `metrics_file`: `metrics.json`;
- `dashboard_file`: `metrics-dashboard.md`;
- compatibility, storage, benchmark, and PostGIS artifact prefixes keyed by source
  SHA.

Release managers should download those artifacts, attach them to the release or
archive them in the release evidence location, and record any skipped drills in
the release notes.

## Promotion rule

A roadmap item is release-backed only when its evidence appears in the release
packet or is explicitly scoped as future/manual. Docs can define the contract;
execution artifacts make the claim.
