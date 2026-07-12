# SPDX-License-Identifier: Apache-2.0
set dotenv-load := true

host := env_var_or_default("QUACKGIS_HOST", "127.0.0.1")
port := env_var_or_default("QUACKGIS_PORT", "5434")
catalog := env_var_or_default("QUACKGIS_CATALOG_PATH", ".tmp/dev/quackgis.ducklake")
data := env_var_or_default("QUACKGIS_DATA_PATH", ".tmp/dev/data")
container_engine := env_var_or_default("CONTAINER_ENGINE", "podman")
duckdb_runtime_image := env_var_or_default("QUACKGIS_DUCKDB_RUNTIME_IMAGE", "localhost/quackgis-duckdb-runtime:dev")
duckdb_bin := env_var_or_default("DUCKDB_BIN", "duckdb")
duckdb_version := env_var_or_default("DUCKDB_VERSION", "1.5.4")
duckdb_home := env_var_or_default("DUCKDB_HOME", ".tmp/duckdb/home")
duckdb_adbc_driver := env_var_or_default("QUACKGIS_DUCKDB_ADBC_DRIVER", ".tmp/duckdb/v" + duckdb_version + "/lib/libduckdb.so")
ref_qgis_url := env_var_or_default("REF_QGIS_URL", "https://github.com/qgis/QGIS")
ref_qgis_branch := env_var_or_default("REF_QGIS_BRANCH", "release-3_44")
ref_duckdb_url := env_var_or_default("REF_DUCKDB_URL", "https://github.com/duckdb/duckdb")
ref_duckdb_branch := env_var_or_default("REF_DUCKDB_BRANCH", "main")
ref_ducklake_spec_url := env_var_or_default("REF_DUCKLAKE_SPEC_URL", "https://github.com/duckdb/ducklake")
ref_ducklake_spec_branch := env_var_or_default("REF_DUCKLAKE_SPEC_BRANCH", "main")
ref_postgis_url := env_var_or_default("REF_POSTGIS_URL", "https://github.com/postgis/postgis")
ref_postgis_branch := env_var_or_default("REF_POSTGIS_BRANCH", "master")
ref_gdal_url := env_var_or_default("REF_GDAL_URL", "https://github.com/OSGeo/gdal")
ref_gdal_branch := env_var_or_default("REF_GDAL_BRANCH", "master")

default:
    just --list

# Install mise-managed tools and bootstrap project-local DuckDB artifacts.
setup:
    mise install
    mise run duckdb-bootstrap

# Verify the local development toolchain expected by repo recipes.
doctor:
    mise --version
    just --version
    cargo --version
    cargo fmt --version
    cargo clippy --version
    mise exec -- duckdb --version
    @test -f '{{duckdb_adbc_driver}}' || { printf '%s\n' 'DuckDB ADBC driver missing; run `just setup`.' >&2; exit 2; }
    @printf "QuackGIS dev toolchain looks usable. Run 'just smoke' for a quick server check.\n"

# Smallest newcomer smoke: run the real DuckDB pgwire workflow.
smoke: duckdb-pgwire-workflow-test

# Alias for a quick local demo that does not require external client binaries.
demo: smoke

# Clone/update all reference repos under ignored .tmp/ref (submodule-init equivalent).
ref-init: ref-clients ref-duckdb-stack ref-postgis-stack

# Clone/update PostgreSQL/PostGIS client source used for trace-driven compatibility.
ref-clients: ref-qgis ref-gdal

# Clone/update DuckDB/DuckLake reference implementation/source.
ref-duckdb-stack: ref-duckdb ref-ducklake-spec

# Clone/update PostGIS reference implementation/source.
ref-postgis-stack: ref-postgis

# Fast-forward/update all reference forks under ignored .tmp/ref.
ref-update: ref-init

# Show status for local reference forks.
ref-status:
    @for repo in qgis gdal duckdb ducklake postgis; do \
        if [ -d ".tmp/ref/$repo/.git" ]; then \
            printf "== %s ==\n" "$repo"; \
            git -C ".tmp/ref/$repo" status --short --branch; \
        else \
            printf "== %s == missing (run 'just ref-init')\n" "$repo"; \
        fi; \
    done

# Clone/update QGIS source for future trace-driven compatibility work.
ref-qgis:
    @just _ref-clone-or-update qgis "{{ref_qgis_url}}" "{{ref_qgis_branch}}"

# Clone/update GDAL/OGR source for PostgreSQL driver trace context.
ref-gdal:
    @just _ref-clone-or-update gdal "{{ref_gdal_url}}" "{{ref_gdal_branch}}"

# Alias: ogr2ogr lives in the GDAL source tree.
ref-ogr2ogr: ref-gdal

# Clone/update DuckDB source.
ref-duckdb:
    @just _ref-clone-or-update duckdb "{{ref_duckdb_url}}" "{{ref_duckdb_branch}}"

# Clone/update official DuckLake reference/spec repository.
ref-ducklake-spec:
    @just _ref-clone-or-update ducklake "{{ref_ducklake_spec_url}}" "{{ref_ducklake_spec_branch}}"

# Clone/update PostGIS source/regression tests.
ref-postgis:
    @just _ref-clone-or-update postgis "{{ref_postgis_url}}" "{{ref_postgis_branch}}"

_ref-clone-or-update name url branch:
    @mkdir -p .tmp/ref
    @if [ -d ".tmp/ref/{{name}}/.git" ]; then \
        printf "Updating .tmp/ref/{{name}} ({{branch}}, ff-only)\n"; \
        git -C ".tmp/ref/{{name}}" fetch --depth 1 origin "{{branch}}"; \
        git -C ".tmp/ref/{{name}}" checkout "{{branch}}"; \
        git -C ".tmp/ref/{{name}}" merge --ff-only FETCH_HEAD; \
    else \
        printf "Cloning {{url}}#{{branch}} -> .tmp/ref/{{name}}\n"; \
        git clone --depth 1 --branch "{{branch}}" "{{url}}" ".tmp/ref/{{name}}"; \
    fi

# Build the server.
build:
    cargo build -p quackgis-server

# Build optimized server binary.
release:
    cargo build -p quackgis-server --release

# Run rustfmt.
fmt:
    cargo fmt --all

# Check formatting.
fmt-check:
    cargo fmt --all -- --check

# Run clippy with repository warning policy.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all default tests.
test:
    cargo test --workspace

# Faster local regression loop that compiles the native-runtime integration gates.
test-fast: arrow-encoder-test
    cargo test -p quackgis-server --lib --test duckdb_adbc_storage --test duckdb_wire_read --test roadmap_profiles --test catalog_contract

# Execute the maintained vendored Arrow-to-pgwire properties and regressions.
arrow-encoder-test:
    cargo test -p arrow-pg

# Install checksum-pinned libduckdb and signed extensions into ignored .tmp.
duckdb-bootstrap duckdb_bin=duckdb_bin:
    @set -eu; \
    duckdb_arg='{{duckdb_bin}}'; \
    duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    python3 scripts/bootstrap_duckdb.py --duckdb-bin "$duckdb_arg"

# Run the out-of-process DuckDB spatial + DuckLake extension engine probe.
duckdb-engine-probe out=".tmp/duckdb-engine/README.md" attach_sql="" duckdb_bin=duckdb_bin:
    @set -eu; \
    out_arg='{{out}}'; \
    attach_arg='{{attach_sql}}'; \
    duckdb_arg='{{duckdb_bin}}'; \
    out_arg="${out_arg#out=}"; \
    attach_arg="${attach_arg#attach_sql=}"; \
    duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    mkdir -p "$(dirname "${out_arg}")"; \
    if [ -n "$attach_arg" ]; then \
        HOME="$duckdb_home_arg" python3 scripts/duckdb_engine_probe.py --duckdb-bin "$duckdb_arg" --out "$out_arg" --attach-sql "$attach_arg"; \
    else \
        HOME="$duckdb_home_arg" python3 scripts/duckdb_engine_probe.py --duckdb-bin "$duckdb_arg" --out "$out_arg"; \
    fi

# Classify and execute the maintained PostGIS subset against pinned DuckDB spatial.
duckdb-spatial-compat-probe out=".tmp/duckdb-spatial/README.md" manifest=".tmp/duckdb-spatial/manifest.json" duckdb_bin=duckdb_bin:
    @set -eu; \
    out_arg='{{out}}'; manifest_arg='{{manifest}}'; duckdb_arg='{{duckdb_bin}}'; \
    out_arg="${out_arg#out=}"; manifest_arg="${manifest_arg#manifest=}"; duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" python3 scripts/duckdb_spatial_compat_probe.py --duckdb-bin "$duckdb_arg" --out "$out_arg" --manifest "$manifest_arg"

# Run the real DuckDB ADBC -> official DuckLake Arrow/mutation/reopen slice.
duckdb-authority-probe workdir=".tmp/duckdb-authority" out=".tmp/duckdb-authority/README.md" manifest=".tmp/duckdb-authority/manifest.json" duckdb_bin=duckdb_bin:
    @set -eu; \
    workdir_arg='{{workdir}}'; \
    out_arg='{{out}}'; \
    manifest_arg='{{manifest}}'; \
    duckdb_arg='{{duckdb_bin}}'; \
    workdir_arg="${workdir_arg#workdir=}"; \
    out_arg="${out_arg#out=}"; \
    manifest_arg="${manifest_arg#manifest=}"; \
    duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" python3 scripts/duckdb_authority_probe.py --duckdb-bin "$duckdb_arg" --workdir "$workdir_arg" --out "$out_arg" --manifest "$manifest_arg"

# Run the real in-process DuckDB ADBC -> official DuckLake slice.
duckdb-adbc-storage-test driver=duckdb_adbc_driver:
    @set -eu; \
    driver_arg='{{driver}}'; \
    driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then \
        echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; \
        exit 2; \
    fi; \
    driver_arg="$(realpath "$driver_arg")"; \
    duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" cargo test -p quackgis-server --test duckdb_adbc_storage -- --ignored --nocapture

# Run the bounded local DuckDB pgwire create/COPY/query/mutation/transaction workflow.
duckdb-pgwire-workflow-test driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; duckdb_arg="$(command -v '{{duckdb_bin}}')"; benchmark_out="$(realpath -m '.tmp/duckdb-current-benchmark/manifest.json')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" DUCKDB_BIN="$duckdb_arg" QUACKGIS_BENCHMARK_OUT="$benchmark_out" \
      cargo test -p quackgis-server --test duckdb_wire_read -- --ignored --nocapture

# Execute the client-neutral DuckDB-derived catalog and geometry identity fixture.
duckdb-catalog-contract-test driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-server --test catalog_contract client_neutral_catalog_contract -- --ignored --exact --nocapture --test-threads=1

# Compare direct DuckDB CLI, ADBC, and pgwire on one deterministic 100k-row profile.
duckdb-current-benchmark driver=duckdb_adbc_driver duckdb_bin=duckdb_bin out=".tmp/duckdb-current-benchmark/manifest.json":
    @set -eu; driver_arg='{{driver}}'; duckdb_arg='{{duckdb_bin}}'; out_arg='{{out}}'; \
    driver_arg="${driver_arg#driver=}"; duckdb_arg="${duckdb_arg#duckdb_bin=}"; out_arg="${out_arg#out=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_arg="$(command -v "$duckdb_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" DUCKDB_BIN="$duckdb_arg" QUACKGIS_BENCHMARK_OUT="$out_arg" \
      cargo test -p quackgis-server --release --test duckdb_wire_read current_duckdb_transport_profile -- --ignored --exact --nocapture --test-threads=1

# Run the parameterized deterministic transport scenario at local/reference scale.
duckdb-transport-profile level="local" rows="1000000" out=".tmp/duckdb-transport-profile/manifest.json" environment="host_process" driver=duckdb_adbc_driver duckdb_bin=duckdb_bin:
    @set -eu; level_arg='{{level}}'; rows_arg='{{rows}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; duckdb_arg='{{duckdb_bin}}'; out_arg='{{out}}'; \
    level_arg="${level_arg#level=}"; rows_arg="${rows_arg#rows=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; duckdb_arg="${duckdb_arg#duckdb_bin=}"; out_arg="${out_arg#out=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_arg="$(command -v "$duckdb_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" DUCKDB_BIN="$duckdb_arg" QUACKGIS_BENCHMARK_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_BENCHMARK_ROWS="$rows_arg" \
      cargo test -p quackgis-server --release --test duckdb_wire_read current_duckdb_transport_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Prove bounded pgwire result streaming with RSS and first-row evidence.
duckdb-result-stream-profile level="smoke" rows="100000" out=".tmp/duckdb-result-stream/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; rows_arg='{{rows}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; rows_arg="${rows_arg#rows=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_PROFILE_ROWS="$rows_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles result_stream_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast native result-stream evidence.
duckdb-result-stream-smoke:
    just duckdb-result-stream-profile level=smoke rows=100000 out=.tmp/duckdb-result-stream/smoke-r100k.json

# Exercise nullable variable-width VARCHAR/BLOB results across native batches.
duckdb-wide-result-profile level="smoke" rows="10000" out=".tmp/duckdb-wide-result/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; rows_arg='{{rows}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; rows_arg="${rows_arg#rows=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_PROFILE_ROWS="$rows_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles wide_result_stream_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast native variable-width result evidence.
duckdb-wide-result-smoke:
    just duckdb-wide-result-profile level=smoke rows=10000 out=.tmp/duckdb-wide-result/smoke-r10k.json

# Measure long-query cancellation latency and explicit session quarantine.
duckdb-cancellation-profile level="smoke" iterations="5" out=".tmp/duckdb-cancellation/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; iterations_arg='{{iterations}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; iterations_arg="${iterations_arg#iterations=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_PROFILE_ITERATIONS="$iterations_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles cancellation_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast native cancellation evidence.
duckdb-cancellation-smoke:
    just duckdb-cancellation-profile level=smoke iterations=5 out=.tmp/duckdb-cancellation/smoke-n5.json

# Exercise reader, writer, and maintenance admission through the native pgwire server.
duckdb-mixed-concurrency-profile level="smoke" out=".tmp/duckdb-mixed-concurrency/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles mixed_class_concurrency_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast mixed-class native admission evidence.
duckdb-mixed-concurrency-smoke:
    just duckdb-mixed-concurrency-profile level=smoke out=.tmp/duckdb-mixed-concurrency/smoke.json

# Prove forced-drain rollback and exact restart state through the actual server process.
duckdb-termination-profile level="smoke" out=".tmp/duckdb-termination/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles termination_atomicity_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast process-level forced-drain/restart evidence.
duckdb-termination-smoke:
    just duckdb-termination-profile level=smoke out=.tmp/duckdb-termination/smoke.json

# Compare direct streaming ADBC ingest with bounded pgwire text COPY.
duckdb-copy-profile level="smoke" rows="10000" out=".tmp/duckdb-copy/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; rows_arg='{{rows}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; rows_arg="${rows_arg#rows=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_PROFILE_ROWS="$rows_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles copy_ingest_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast native COPY evidence.
duckdb-copy-smoke:
    just duckdb-copy-profile level=smoke rows=10000 out=.tmp/duckdb-copy/smoke-r10k.json

# Create an offline, exact-path local DuckLake backup with a checksum manifest.
duckdb-local-backup catalog=catalog data=data out=".tmp/duckdb-backup":
    python3 scripts/duckdb_local_backup.py backup --catalog "{{catalog}}" --data-root "{{data}}" --destination "{{out}}"

# Restore a verified local DuckLake backup to its exact original paths.
duckdb-local-restore backup catalog=catalog data=data:
    python3 scripts/duckdb_local_backup.py restore --backup "{{backup}}" --catalog "{{catalog}}" --data-root "{{data}}"

# Compatibility alias for the original read-only checkpoint recipe.
duckdb-pgwire-read-test: duckdb-pgwire-workflow-test

# Compile and unit-test the DuckDB ADBC boundary without requiring libduckdb.
duckdb-adbc-compile-check:
    cargo test -p quackgis-server --lib
    cargo test -p quackgis-server --test duckdb_adbc_storage --no-run

# Run the curated PostGIS subset through the real DuckDB pgwire workflow.
postgis-regress: duckdb-pgwire-workflow-test

# Run nextest when installed by mise.
nextest:
    cargo nextest run --workspace

# Validate the common roadmap evidence envelope emitted by native profiles.
evidence-manifest-check manifest=".tmp/duckdb-current-benchmark/manifest.json":
    @manifest_arg='{{manifest}}'; manifest_arg="${manifest_arg#manifest=}"; python3 scripts/evidence_manifest_check.py "$manifest_arg"

# Full local verification gate.
check: fmt-check clippy test

# Faster local verification gate for edit/probe triage.
check-fast: fmt-check clippy test-fast

# Run the same gate used by GitHub Actions CI.
ci: check-fast project-contract-check duckdb-adbc-compile-check duckdb-adbc-storage-test duckdb-pgwire-workflow-test duckdb-catalog-contract-test duckdb-result-stream-smoke duckdb-wide-result-smoke duckdb-cancellation-smoke duckdb-mixed-concurrency-smoke duckdb-termination-smoke duckdb-copy-smoke evidence-manifest-check probe-static-check runtime-static-check kind-static-check

# Run the dev QuackGIS server on QUACKGIS_HOST/QUACKGIS_PORT.
server:
    mkdir -p "$(dirname '{{catalog}}')" "{{data}}"
    cargo run -p quackgis-server -- --host {{host}} --port {{port}} --catalog-path "{{catalog}}" --data-path "{{data}}"

# Connect with psql to a running dev server.
psql:
    psql -h {{host}} -p {{port}} -U postgres -d quackgis

# Remove local dev DuckLake catalog/data.
clean-dev:
    rm -rf .tmp/dev

# Static validation for maintained helper scripts.
probe-static-check:
    mkdir -p .tmp/pycache
    PYTHONPYCACHEPREFIX=.tmp/pycache python3 -m py_compile scripts/*.py deploy/kind/render.py scripts/tests/test_duckdb_authority_probe.py scripts/tests/test_duckdb_engine_probe.py scripts/tests/test_duckdb_runtime_static_check.py scripts/tests/test_duckdb_spatial_compat_probe.py scripts/tests/test_evidence_manifest_check.py scripts/tests/test_kind_render.py
    python3 scripts/tests/test_duckdb_authority_probe.py
    python3 scripts/tests/test_duckdb_engine_probe.py
    python3 scripts/tests/test_duckdb_spatial_compat_probe.py
    python3 scripts/tests/test_duckdb_runtime_static_check.py
    python3 scripts/tests/test_duckdb_local_backup.py
    python3 scripts/tests/test_evidence_manifest_check.py

# Validate digest pinning, generated secrets, and the DuckDB-only Kind shape.
kind-static-check:
    python3 deploy/kind/render.py --check
    python3 scripts/tests/test_kind_render.py

# Validate maintained documentation links, commands, and spatial claims.
project-contract-check:
    python3 scripts/project_contract_check.py

# Compatibility alias for the sole maintained DuckDB runtime image guard.
runtime-static-check: duckdb-runtime-static-check

# Guard the separate native DuckDB runtime against online installs or missing artifacts.
duckdb-runtime-static-check:
    python3 scripts/duckdb_runtime_static_check.py deploy/Containerfile.duckdb-runtime
    python3 scripts/tests/test_duckdb_runtime_static_check.py

# Assemble a verified Linux x86_64 DuckDB runtime context under ignored .tmp.
duckdb-runtime-context:
    cargo build -p quackgis-server --release
    @duckdb_path="$(mise exec -- which duckdb)"; dirty_flag=""; \
    if [ "${QUACKGIS_ALLOW_DIRTY_RUNTIME:-0}" = 1 ]; then dirty_flag="--allow-dirty"; fi; \
    python3 scripts/prepare_duckdb_runtime.py $dirty_flag --server target/release/quackgis-server --duckdb-bin "$duckdb_path"

# Build the immutable local DuckDB evaluation runtime image.
duckdb-runtime-image: duckdb-runtime-static-check duckdb-runtime-context
    {{container_engine}} build -t {{duckdb_runtime_image}} -f deploy/Containerfile.duckdb-runtime .tmp/duckdb-runtime

# Prove pinned extensions load with all container networking disabled.
duckdb-runtime-offline-smoke: duckdb-runtime-image
    {{container_engine}} run --rm --network none --entrypoint /usr/local/bin/duckdb {{duckdb_runtime_image}} -csv -noheader :memory: -c "LOAD spatial; LOAD ducklake; SELECT ST_AsText(ST_Point(1, 2));"
    @set -eu; \
    container_id="$({{container_engine}} run -d --network none {{duckdb_runtime_image}})"; \
    trap '{{container_engine}} rm -f "$container_id" >/dev/null 2>&1 || true' EXIT; \
    sleep 3; \
    if ! {{container_engine}} exec "$container_id" /bin/sh -c 'kill -0 1'; then \
        {{container_engine}} logs "$container_id"; \
        exit 1; \
    fi; \
    {{container_engine}} logs "$container_id"; \
    printf 'duckdb_runtime_server_smoke_ok\n'
