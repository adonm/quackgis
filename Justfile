# SPDX-License-Identifier: Apache-2.0
set dotenv-load := true

host := env_var_or_default("QUACKGIS_HOST", "127.0.0.1")
port := env_var_or_default("QUACKGIS_PORT", "5434")
catalog := env_var_or_default("QUACKGIS_CATALOG_PATH", ".tmp/dev/quackgis.ducklake")
data := env_var_or_default("QUACKGIS_DATA_PATH", ".tmp/dev/data")
container_engine := env_var_or_default("CONTAINER_ENGINE", "")
duckdb_runtime_image := env_var_or_default("QUACKGIS_DUCKDB_RUNTIME_IMAGE", "localhost/quackgis-duckdb-runtime:dev")
kind_client_image := env_var_or_default("QUACKGIS_KIND_CLIENT_IMAGE", "localhost/quackgis-kind-clients:dev")
duckdb_bin := env_var_or_default("DUCKDB_BIN", "duckdb")
duckdb_version := env_var_or_default("DUCKDB_VERSION", "1.5.4")
duckdb_home := env_var_or_default("DUCKDB_HOME", ".tmp/duckdb/home")
duckdb_adbc_driver := env_var_or_default("QUACKGIS_DUCKDB_ADBC_DRIVER", ".tmp/duckdb/v" + duckdb_version + "/lib/libduckdb.so")
pinned_ducklake_extension := env_var_or_default("QUACKGIS_DUCKLAKE_EXTENSION", ".tmp/ref/quackgis-ducklake/build/release/extension/ducklake/ducklake.duckdb_extension")
pinned_ducklake_extension_sha256 := env_var_or_default("QUACKGIS_DUCKLAKE_EXTENSION_SHA256", "046e73c864b4403e73beddc39addc72a370dfbe633e2287181a1c0cdd37b5b94")
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
    python3 scripts/project_doctor.py
    @test -f '{{duckdb_adbc_driver}}' || printf '%s\n' 'note: DuckDB ADBC driver missing; run `just setup`.'

# Fail unless the core build/test toolchain is installed.
doctor-core:
    python3 scripts/project_doctor.py --check core
    cargo fmt --version
    cargo clippy --version
    @test -f '{{duckdb_adbc_driver}}' || { printf '%s\n' 'DuckDB ADBC driver missing; run `just setup`.' >&2; exit 2; }

# Fail unless Kind, kubectl, TLS tooling, and one usable container engine are installed.
doctor-kind: doctor-core
    python3 scripts/project_doctor.py --check kind

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
test-fast: arrow-encoder-test iroh-protocol-test iroh-direct-smoke iroh-custom-relay-smoke
    cargo test -p quackgis-server --lib --test duckdb_adbc_storage --test duckdb_wire_read --test roadmap_profiles --test catalog_contract --test iroh_direct

# Execute the maintained vendored Arrow-to-pgwire properties and regressions.
arrow-encoder-test:
    cargo test -p arrow-pg

# Verify signed I0 leases, key proofs, stream types, and relay policy.
iroh-protocol-test:
    cargo test -p quackgis-edge --lib

# Exercise bootstrap, worker, and tiny-client endpoints over a local direct iroh path.
iroh-direct-smoke:
    cargo test -p quackgis-edge --test direct_path

# Force the authenticated bridge and adaptive codec through one deterministic custom relay.
iroh-custom-relay-smoke:
    cargo test -p quackgis-edge --test relay_path custom_relay_forces_application_bytes_off_direct_paths -- --exact

# Exercise omitted relay configuration against iroh's real public preset (requires outbound network).
iroh-public-relay-smoke:
    cargo test -p quackgis-edge --test relay_path omitted_configuration_uses_public_relay_for_reconnect -- --ignored --exact --nocapture --test-threads=1

# Differentially run result/type/error/portal/transaction/COPY/cancellation oracles over direct TCP and local iroh.
iroh-duckdb-smoke driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-server --test iroh_direct duckdb_pgwire_oracles_pass_through_local_iroh -- --ignored --exact --nocapture --test-threads=1

# Run the same native DuckDB differential oracle over a relay-only custom iroh path.
iroh-duckdb-relay-smoke driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-server --test iroh_direct duckdb_pgwire_oracles_pass_through_forced_custom_relay -- --ignored --exact --nocapture --test-threads=1

# Run the native DuckDB differential oracle through iroh's public relay preset (requires outbound network).
iroh-duckdb-public-relay-smoke driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-server --test iroh_direct duckdb_pgwire_oracles_pass_through_public_default_relay -- --ignored --exact --nocapture --test-threads=1

# Measure TCP, direct iroh, and forced-relay off/auto transport behavior and enforce I0 budgets.
iroh-transport-profile level="smoke" bytes="" out=".tmp/iroh-transport-profile/smoke.json":
    @set -eu; level_arg='{{level}}'; bytes_arg='{{bytes}}'; out_arg='{{out}}'; \
    level_arg="${level_arg#level=}"; bytes_arg="${bytes_arg#bytes=}"; out_arg="${out_arg#out=}"; out_arg="$(realpath -m "$out_arg")"; \
    if [ -n "$bytes_arg" ]; then export QUACKGIS_IROH_PROFILE_BYTES="$bytes_arg"; fi; \
    QUACKGIS_IROH_PROFILE_LEVEL="$level_arg" QUACKGIS_IROH_PROFILE_OUT="$out_arg" \
      cargo test --release -p quackgis-edge --test transport_profile direct_and_relay_transport_profile -- --ignored --exact --nocapture --test-threads=1

# Install checksum-pinned libduckdb and signed extensions into ignored .tmp.
duckdb-bootstrap duckdb_bin=duckdb_bin:
    @set -eu; \
    duckdb_arg='{{duckdb_bin}}'; \
    duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    python3 scripts/bootstrap_duckdb.py --duckdb-bin "$duckdb_arg"

# Validate the tracked source, patch, tool, and accepted artifact pins for DuckLake.
ducklake-pinned-source-check:
    python3 scripts/build_pinned_ducklake.py --check

# Build and test the accepted DuckLake identity extension from tracked source pins.
ducklake-pinned-build:
    mise x aqua:Kitware/CMake@4.3.3 aqua:ninja-build/ninja@1.13.2 -- python3 scripts/build_pinned_ducklake.py

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

# Prove durable DuckLake table/column identity across rename, reopen, and name reuse.
duckdb-catalog-identity-test workdir=".tmp/duckdb-catalog-identity" out=".tmp/duckdb-catalog-identity/README.md" manifest=".tmp/duckdb-catalog-identity/manifest.json" duckdb_bin=duckdb_bin:
    @set -eu; workdir_arg='{{workdir}}'; out_arg='{{out}}'; manifest_arg='{{manifest}}'; duckdb_arg='{{duckdb_bin}}'; \
    workdir_arg="${workdir_arg#workdir=}"; out_arg="${out_arg#out=}"; manifest_arg="${manifest_arg#manifest=}"; duckdb_arg="${duckdb_arg#duckdb_bin=}"; \
    duckdb_arg="$(command -v "$duckdb_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" python3 scripts/duckdb_catalog_identity_probe.py --duckdb-bin "$duckdb_arg" --workdir "$workdir_arg" --out "$out_arg" --manifest "$manifest_arg"

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

# Run the supported pinned DuckLake identity and PostgreSQL catalog lifecycle contract.
duckdb-pinned-ducklake-test extension=pinned_ducklake_extension sha256=pinned_ducklake_extension_sha256 driver=duckdb_adbc_driver:
    @set -eu; \
    extension_arg='{{extension}}'; sha256_arg='{{sha256}}'; driver_arg='{{driver}}'; \
    extension_arg="${extension_arg#extension=}"; sha256_arg="${sha256_arg#sha256=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$extension_arg" ] || [ -L "$extension_arg" ]; then echo 'Pinned DuckLake extension must be a non-symlink file; see docs/PINNED_DUCKLAKE.md' >&2; exit 2; fi; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    extension_arg="$(realpath "$extension_arg")"; driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_DUCKLAKE_EXTENSION="$extension_arg" QUACKGIS_DUCKLAKE_EXTENSION_SHA256="$sha256_arg" \
      cargo test -p quackgis-server --test pinned_ducklake pinned_ducklake_column_identity_contract -- --ignored --exact --nocapture

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

# Prove required TLS, SCRAM, plaintext denial, and restart-based credential rotation.
duckdb-tls-rotation-profile level="smoke" out=".tmp/duckdb-tls-rotation/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles tls_required_rotation_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast actual-process TLS and credential-rotation evidence.
duckdb-tls-rotation-smoke:
    just duckdb-tls-rotation-profile level=smoke out=.tmp/duckdb-tls-rotation/smoke.json

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

# Compare maintained WKB/bbox pruning with native GEOMETRY statistics and exact rechecks.
duckdb-spatial-scan-profile level="smoke" rows="100000" out=".tmp/duckdb-spatial-scan/manifest.json" environment="host_process" driver=duckdb_adbc_driver:
    @set -eu; level_arg='{{level}}'; rows_arg='{{rows}}'; out_arg='{{out}}'; environment_arg='{{environment}}'; driver_arg='{{driver}}'; \
    level_arg="${level_arg#level=}"; rows_arg="${rows_arg#rows=}"; out_arg="${out_arg#out=}"; environment_arg="${environment_arg#environment=}"; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; out_arg="$(realpath -m "$out_arg")"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" QUACKGIS_PROFILE_OUT="$out_arg" \
      QUACKGIS_EVIDENCE_LEVEL="$level_arg" QUACKGIS_EXECUTION_ENVIRONMENT="$environment_arg" QUACKGIS_PROFILE_ROWS="$rows_arg" \
      cargo test -p quackgis-server --release --test roadmap_profiles spatial_scan_profile -- --ignored --exact --nocapture --test-threads=1; \
    python3 scripts/evidence_manifest_check.py "$out_arg"

# Required fast native spatial pruning and exact-result evidence.
duckdb-spatial-scan-smoke:
    just duckdb-spatial-scan-profile level=smoke rows=100000 out=.tmp/duckdb-spatial-scan/smoke-r100k.json

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
ci: check-fast project-contract-check duckdb-adbc-compile-check duckdb-adbc-storage-test duckdb-pgwire-workflow-test iroh-duckdb-smoke iroh-duckdb-relay-smoke iroh-transport-profile rest-postgrest-smoke duckdb-catalog-contract-test duckdb-catalog-identity-test duckdb-result-stream-smoke duckdb-wide-result-smoke duckdb-cancellation-smoke duckdb-mixed-concurrency-smoke duckdb-termination-smoke duckdb-tls-rotation-smoke duckdb-copy-smoke duckdb-spatial-scan-smoke evidence-manifest-check probe-static-check runtime-static-check kind-static-check

# Run the dev QuackGIS server on QUACKGIS_HOST/QUACKGIS_PORT.
server:
    mkdir -p "$(dirname '{{catalog}}')" "{{data}}"
    cargo run -p quackgis-server -- --host {{host}} --port {{port}} --catalog-path "{{catalog}}" --data-path "{{data}}"

# Run the authenticated read-only PostgREST-compatible sidecar from its environment configuration.
rest-server:
    cargo run -p quackgis-rest

# Verify the pinned pg-rest-server parser/query extension without native DuckDB.
rest-check:
    cargo test -p quackgis-rest

# Exercise the PostgREST read subset and QuackGIS WKB extension through actual pgwire.
rest-postgrest-smoke driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-rest tests::actual_postgrest_compat_and_quackgis_extensions -- --ignored --exact

# Prove passwordless REST connects only to the loopback edge-preauthenticated backend.
rest-edge-preauth-smoke driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      cargo test -p quackgis-rest tests::edge_preauthenticated_rest_connector_uses_no_database_password -- --ignored --exact

# Exercise REST cache invalidation through pinned catalog/security epochs.
rest-shared-epoch-smoke extension=pinned_ducklake_extension sha256=pinned_ducklake_extension_sha256 driver=duckdb_adbc_driver:
    @set -eu; driver_arg='{{driver}}'; driver_arg="${driver_arg#driver=}"; extension_arg='{{extension}}'; extension_arg="${extension_arg#extension=}"; sha_arg='{{sha256}}'; sha_arg="${sha_arg#sha256=}"; \
    if [ ! -f "$driver_arg" ]; then echo 'DuckDB ADBC driver is missing; run `mise run duckdb-bootstrap`' >&2; exit 2; fi; \
    if [ ! -f "$extension_arg" ] || [ -z "$sha_arg" ]; then echo 'supported pinned DuckLake extension is required' >&2; exit 2; fi; \
    driver_arg="$(realpath "$driver_arg")"; extension_arg="$(realpath "$extension_arg")"; duckdb_home_arg="$(realpath -m '{{duckdb_home}}')"; \
    HOME="$duckdb_home_arg" QUACKGIS_DUCKDB_ADBC_DRIVER="$driver_arg" \
      QUACKGIS_DUCKLAKE_EXTENSION="$extension_arg" QUACKGIS_DUCKLAKE_EXTENSION_SHA256="$sha_arg" \
      cargo test -p quackgis-rest tests::shared_catalog_epochs_invalidate_rest_caches -- --ignored --exact

# Connect with psql to a running dev server.
psql:
    psql -h {{host}} -p {{port}} -U postgres -d quackgis

# Remove local dev DuckLake catalog/data.
clean-dev:
    rm -rf .tmp/dev

# Static validation for maintained helper scripts.
probe-static-check:
    mkdir -p .tmp/pycache
    PYTHONPYCACHEPREFIX=.tmp/pycache python3 -m py_compile scripts/*.py deploy/kind/render.py scripts/tests/test_build_pinned_ducklake.py scripts/tests/test_duckdb_authority_probe.py scripts/tests/test_duckdb_catalog_identity_probe.py scripts/tests/test_duckdb_engine_probe.py scripts/tests/test_duckdb_runtime_static_check.py scripts/tests/test_duckdb_spatial_compat_probe.py scripts/tests/test_evidence_manifest_check.py scripts/tests/test_kind_render.py scripts/tests/test_prepare_duckdb_runtime.py scripts/tests/test_project_doctor.py
    python3 scripts/tests/test_build_pinned_ducklake.py
    python3 scripts/tests/test_duckdb_authority_probe.py
    python3 scripts/tests/test_duckdb_catalog_identity_probe.py
    python3 scripts/tests/test_duckdb_engine_probe.py
    python3 scripts/tests/test_duckdb_spatial_compat_probe.py
    python3 scripts/tests/test_duckdb_runtime_static_check.py
    python3 scripts/tests/test_prepare_duckdb_runtime.py
    python3 scripts/tests/test_duckdb_local_backup.py
    python3 scripts/tests/test_evidence_manifest_check.py
    python3 scripts/tests/test_project_doctor.py

# Validate digest pinning, generated secrets, and the DuckDB-only Kind shape.
kind-static-check:
    python3 deploy/kind/render.py --check
    python3 scripts/tests/test_kind_render.py
    python3 scripts/tests/test_project_doctor.py
    sh -n deploy/kind/up.sh deploy/kind/load-image.sh deploy/kind/down.sh deploy/kind/rotate.sh deploy/kind/rotate-rest-jwt.sh deploy/kind/rest-gates.sh

# Build the non-root psql/psycopg/OGR qualification image with the selected engine.
kind-client-image:
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    "$engine" build -t {{kind_client_image}} -f deploy/Containerfile.kind-clients .; \
    "$engine" run --rm --entrypoint /bin/sh {{kind_client_image}} -c \
      'set -eu; id; test "$(psql --version)" = "psql (PostgreSQL) 18.3"; python3 -c "import psycopg; assert psycopg.__version__ == \"3.2.13\""; ogrinfo --version | grep -F "GDAL 3.11.5"; ogr2ogr --version | grep -F "GDAL 3.11.5"; ogrinfo --formats | grep -F "PostgreSQL -vector-"; printf "kind_client_versions_ok psql=18.3 psycopg=3.2.13 ogr=3.11.5\n"'

# Build both images used by the local Kind qualification topology.
kind-local-images: duckdb-runtime-image kind-client-image

# Create/update the local Kind cluster, load both local images, and wait for the packaged edge path.
kind-up-local: doctor-kind kind-local-images
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    CONTAINER_ENGINE="$engine" \
    QUACKGIS_RUNTIME_LOAD_IMAGE='{{duckdb_runtime_image}}' \
    QUACKGIS_CLIENT_LOAD_IMAGE='{{kind_client_image}}' \
    deploy/kind/up.sh

# Run pinned pgwire clients plus both role-aware REST replicas and failover gates.
kind-client-gates:
    @set -eu; export KUBECONFIG="${KUBECONFIG:-$PWD/.tmp/kind/kubeconfig}"; \
    kubectl delete -f .tmp/kind/rendered/clients.yaml --ignore-not-found >/dev/null; \
    kubectl apply -f .tmp/kind/rendered/clients.yaml; \
    for job in quackgis-psql quackgis-psycopg quackgis-ogr quackgis-direct-denied quackgis-plaintext-denied quackgis-uncredentialed-denied; do \
      if ! kubectl -n quackgis wait --for=condition=complete "job/$job" --timeout=2m; then \
        kubectl -n quackgis logs "job/$job" --all-containers=true || true; exit 1; \
      fi; \
      kubectl -n quackgis logs "job/$job" --all-containers=true; \
    done; \
    deploy/kind/rest-gates.sh

# Run the normal copied-data matrix, then qualify the pinned headless QGIS provider.
kind-qgis-gate: kind-client-gates
    @set -eu; export KUBECONFIG="${KUBECONFIG:-$PWD/.tmp/kind/kubeconfig}"; \
    kubectl delete -f .tmp/kind/rendered/qgis.yaml --ignore-not-found >/dev/null; \
    kubectl apply -f .tmp/kind/rendered/qgis.yaml; \
    if ! kubectl -n quackgis wait --for=condition=complete job/quackgis-qgis --timeout=5m; then \
      kubectl -n quackgis logs job/quackgis-qgis --all-containers=true || true; exit 1; \
    fi; \
    kubectl -n quackgis logs job/quackgis-qgis --all-containers=true

# Prove both REST Pods, role denial, Service endpoints, and one-Pod failover.
kind-rest-gates:
    deploy/kind/rest-gates.sh

# Replace the packaged Pod in shutdown order, then prove every gate reconnects.
kind-restart-gate:
    @set -eu; export KUBECONFIG="${KUBECONFIG:-$PWD/.tmp/kind/kubeconfig}"; \
    start="$(date +%s%3N)"; \
    kubectl -n quackgis rollout restart statefulset/quackgis; \
    kubectl -n quackgis rollout status statefulset/quackgis --timeout=3m; \
    end="$(date +%s%3N)"; printf 'kind_restart_ok elapsed_ms=%s\n' "$((end-start))"; \
    just kind-client-gates

# Rotate packaged mTLS and iroh keys, reject the old certificate, then rerun clients.
kind-secret-rotation-gate:
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    CONTAINER_ENGINE="$engine" deploy/kind/rotate.sh; \
    if just kind-client-gates; then \
      rm -rf .tmp/kind/previous-tls .tmp/kind/previous-edge; \
      printf 'kind_secret_rotation_ok old_client=denied current_gates=passed\n'; \
    else \
      printf 'rotation gate failed; previous material retained under .tmp/kind\n' >&2; exit 1; \
    fi

# Replace the shared packaged JWT key, recreate both replicas, and deny old-key tokens per Pod.
kind-rest-jwt-rotation-gate:
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    CONTAINER_ENGINE="$engine" deploy/kind/rotate-rest-jwt.sh; \
    rm -f .tmp/kind/previous-rest-jwt; \
    printf 'kind_rest_jwt_rotation_ok old_tokens=denied replicas=2\n'

# Delete the named local Kind cluster using the auto-selected provider.
kind-down:
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    CONTAINER_ENGINE="$engine" deploy/kind/down.sh

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
    cargo build -p quackgis-rest --release
    cargo build -p quackgis-edge --release --bins
    @duckdb_path="$(mise exec -- which duckdb)"; dirty_flag=""; \
    if [ "${QUACKGIS_ALLOW_DIRTY_RUNTIME:-0}" = 1 ]; then dirty_flag="--allow-dirty"; fi; \
    python3 scripts/prepare_duckdb_runtime.py $dirty_flag --server target/release/quackgis-server --rest target/release/quackgis-rest --edge-bin-dir target/release --duckdb-bin "$duckdb_path" --ducklake-extension '{{pinned_ducklake_extension}}'

# Build the immutable local DuckDB evaluation runtime image.
duckdb-runtime-image: duckdb-runtime-static-check duckdb-runtime-context
    @set -eu; engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    "$engine" build -t {{duckdb_runtime_image}} -f deploy/Containerfile.duckdb-runtime .tmp/duckdb-runtime

# Prove pinned extensions load with all container networking disabled.
duckdb-runtime-offline-smoke: duckdb-runtime-image
    @set -eu; \
    engine="$(CONTAINER_ENGINE='{{container_engine}}' python3 scripts/project_doctor.py --container-engine)"; \
    for binary in quackgis-rest quackgis-bootstrap quackgis-worker-edge quackgis-client quackgis-keygen; do \
      "$engine" run --rm --network none --entrypoint "/usr/local/bin/$binary" {{duckdb_runtime_image}} --version; \
    done; \
    "$engine" run --rm --network none --entrypoint /usr/local/bin/duckdb {{duckdb_runtime_image}} -unsigned -csv -noheader :memory: -c "LOAD spatial; LOAD ducklake; SELECT ST_AsText(ST_Point(1, 2));"; \
    container_id="$("$engine" run -d --network none {{duckdb_runtime_image}})"; \
    trap '"$engine" rm -f "$container_id" >/dev/null 2>&1 || true' EXIT; \
    sleep 3; \
    if ! "$engine" exec "$container_id" /bin/sh -c 'kill -0 1'; then \
        "$engine" logs "$container_id"; \
        exit 1; \
    fi; \
    "$engine" logs "$container_id"; \
    printf 'duckdb_runtime_server_smoke_ok\n'
