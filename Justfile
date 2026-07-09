# SPDX-License-Identifier: Apache-2.0
set dotenv-load := true

host := env_var_or_default("QUACKGIS_HOST", "127.0.0.1")
port := env_var_or_default("QUACKGIS_PORT", "5434")
smoke_host := env_var_or_default("QUACKGIS_SMOKE_HOST", "127.0.0.1")
smoke_port := env_var_or_default("QUACKGIS_SMOKE_PORT", "15434")
catalog := env_var_or_default("QUACKGIS_CATALOG_PATH", ".tmp/dev/quackgis.db")
data := env_var_or_default("QUACKGIS_DATA_PATH", ".tmp/dev/data")
martin_bin := env_var_or_default("MARTIN_BIN", ".tmp/bin/martin")
martin_version := env_var_or_default("MARTIN_VERSION", "1.11.0")
martin_port := env_var_or_default("MARTIN_PORT", "3000")
qgis_image := env_var_or_default("QGIS_IMAGE", "docker.io/qgis/qgis:ltr-questing")
geoserver_image := env_var_or_default("GEOSERVER_IMAGE", "docker.osgeo.org/geoserver:3.0.0")
postgis_image := env_var_or_default("POSTGIS_IMAGE", "docker.io/postgis/postgis:16-3.4")
osm_extract_url := env_var_or_default("OSM_EXTRACT_URL", "https://download.geofabrik.de/europe/monaco-latest.osm.pbf")
osm_point_limit := env_var_or_default("OSM_POINT_LIMIT", "50")
osm_line_limit := env_var_or_default("OSM_LINE_LIMIT", "50")
osm_polygon_limit := env_var_or_default("OSM_POLYGON_LIMIT", "50")
container_engine := env_var_or_default("CONTAINER_ENGINE", "podman")
kind_cluster := env_var_or_default("KIND_CLUSTER", "quackgis")
quackgis_image := env_var_or_default("QUACKGIS_IMAGE", "localhost/quackgis:dev")
linkerd_iptables_mode := env_var_or_default("LINKERD_IPTABLES_MODE", "nft")
qps_deep_factor := env_var_or_default("QPS_DEEP_FACTOR", "10000")
qps_deep_workers := env_var_or_default("QPS_DEEP_WORKERS", "32")
qps_deep_queries := env_var_or_default("QPS_DEEP_QUERIES", "640")
qps_deep_replicas := env_var_or_default("QPS_DEEP_REPLICAS", "4")
qps_deep_min_instances := env_var_or_default("QPS_DEEP_MIN_INSTANCES", "3")
qps_deep_min_qps := env_var_or_default("QPS_DEEP_MIN_QPS", "1.0")
qps_deep_disk_budget_gib := env_var_or_default("QPS_DEEP_DISK_BUDGET_GIB", "1024")
qps_shared_catalog_refresh_ms := env_var_or_default("QPS_SHARED_CATALOG_REFRESH_MS", "60000")
ref_datafusion_postgres_url := env_var_or_default("REF_DATAFUSION_POSTGRES_URL", "https://github.com/adonm/datafusion-postgres")
ref_datafusion_postgres_branch := env_var_or_default("REF_DATAFUSION_POSTGRES_BRANCH", "quackgis/fixes")
ref_datafusion_postgres_upstream_url := env_var_or_default("REF_DATAFUSION_POSTGRES_UPSTREAM_URL", "https://github.com/datafusion-contrib/datafusion-postgres")
ref_datafusion_postgres_upstream_branch := env_var_or_default("REF_DATAFUSION_POSTGRES_UPSTREAM_BRANCH", "master")
ref_sedona_url := env_var_or_default("REF_SEDONA_URL", "https://github.com/adonm/sedona-db.git")
ref_sedona_branch := env_var_or_default("REF_SEDONA_BRANCH", "quackgis/df54")
ref_sedona_upstream_url := env_var_or_default("REF_SEDONA_UPSTREAM_URL", "https://github.com/apache/sedona-db.git")
ref_sedona_upstream_branch := env_var_or_default("REF_SEDONA_UPSTREAM_BRANCH", "main")
ref_ducklake_url := env_var_or_default("REF_DUCKLAKE_URL", "https://github.com/adonm/datafusion-ducklake")
ref_ducklake_branch := env_var_or_default("REF_DUCKLAKE_BRANCH", "main")
ref_ducklake_upstream_url := env_var_or_default("REF_DUCKLAKE_UPSTREAM_URL", "https://github.com/datafusion-contrib/datafusion-ducklake")
ref_ducklake_upstream_branch := env_var_or_default("REF_DUCKLAKE_UPSTREAM_BRANCH", "main")
ref_martin_url := env_var_or_default("REF_MARTIN_URL", "https://github.com/maplibre/martin")
ref_martin_branch := env_var_or_default("REF_MARTIN_BRANCH", "main")
ref_qgis_url := env_var_or_default("REF_QGIS_URL", "https://github.com/qgis/QGIS")
ref_qgis_branch := env_var_or_default("REF_QGIS_BRANCH", "release-3_44")
ref_geoserver_url := env_var_or_default("REF_GEOSERVER_URL", "https://github.com/geoserver/geoserver")
ref_geoserver_branch := env_var_or_default("REF_GEOSERVER_BRANCH", "2.26.x")
ref_duckdb_url := env_var_or_default("REF_DUCKDB_URL", "https://github.com/duckdb/duckdb")
ref_duckdb_branch := env_var_or_default("REF_DUCKDB_BRANCH", "main")
ref_ducklake_spec_url := env_var_or_default("REF_DUCKLAKE_SPEC_URL", "https://github.com/duckdb/ducklake")
ref_ducklake_spec_branch := env_var_or_default("REF_DUCKLAKE_SPEC_BRANCH", "main")
ref_pg_ducklake_url := env_var_or_default("REF_PG_DUCKLAKE_URL", "https://github.com/duckdb/pg_ducklake")
ref_pg_ducklake_branch := env_var_or_default("REF_PG_DUCKLAKE_BRANCH", "main")
ref_postgis_url := env_var_or_default("REF_POSTGIS_URL", "https://github.com/postgis/postgis")
ref_postgis_branch := env_var_or_default("REF_POSTGIS_BRANCH", "master")
ref_gdal_url := env_var_or_default("REF_GDAL_URL", "https://github.com/OSGeo/gdal")
ref_gdal_branch := env_var_or_default("REF_GDAL_BRANCH", "master")
ref_sqlite_url := env_var_or_default("REF_SQLITE_URL", "https://github.com/sqlite/sqlite")
ref_sqlite_branch := env_var_or_default("REF_SQLITE_BRANCH", "master")

default:
    just --list

# Install mise-managed tools and project-local helper binaries.
setup:
    mise install
    just install-martin

# Verify the local development toolchain expected by repo recipes.
doctor:
    mise --version
    just --version
    cargo --version
    cargo fmt --version
    cargo clippy --version
    @printf "QuackGIS dev toolchain looks usable. Run 'just smoke' for a quick server check.\n"

# Smallest newcomer smoke: start the test server and run one spatial pgwire query.
smoke:
    cargo test -p quackgis-server --test wire_spatial wire_spatial_queries_execute -- --nocapture

# Alias for a quick local demo that does not require external client binaries.
demo: smoke

# Clone/update all reference repos under ignored .tmp/ref (submodule-init equivalent).
ref-init: ref-core ref-clients ref-duckdb-stack ref-postgis-stack ref-storage-refs

# Clone/update Rust engine forks/upstreams used by QuackGIS.
ref-core: ref-datafusion-postgres ref-datafusion-postgres-upstream ref-sedona ref-sedona-upstream ref-ducklake ref-ducklake-upstream

# Clone/update PostgreSQL/PostGIS client source used for trace-driven compatibility.
ref-clients: ref-martin ref-qgis ref-geoserver ref-gdal

# Clone/update DuckDB/DuckLake reference implementation/source.
ref-duckdb-stack: ref-duckdb ref-ducklake-spec ref-pg-ducklake

# Clone/update PostGIS reference implementation/source.
ref-postgis-stack: ref-postgis

# Clone/update storage-adjacent reference implementations.
ref-storage-refs: ref-sqlite

# Fast-forward/update all reference forks under ignored .tmp/ref.
ref-update: ref-init

# Show status for local reference forks.
ref-status:
    @for repo in datafusion-postgres datafusion-postgres-upstream sedona-db sedona-db-upstream datafusion-ducklake datafusion-ducklake-upstream martin qgis geoserver gdal duckdb ducklake pg-ducklake postgis sqlite; do \
        if [ -d ".tmp/ref/$repo/.git" ]; then \
            printf "== %s ==\n" "$repo"; \
            git -C ".tmp/ref/$repo" status --short --branch; \
        else \
            printf "== %s == missing (run 'just ref-init')\n" "$repo"; \
        fi; \
    done

# Clone/update the datafusion-postgres fork used by Cargo.toml.
ref-datafusion-postgres:
    @just _ref-clone-or-update datafusion-postgres "{{ref_datafusion_postgres_url}}" "{{ref_datafusion_postgres_branch}}"

# Clone/update upstream datafusion-postgres for comparison/rebase context.
ref-datafusion-postgres-upstream:
    @just _ref-clone-or-update datafusion-postgres-upstream "{{ref_datafusion_postgres_upstream_url}}" "{{ref_datafusion_postgres_upstream_branch}}"

# Clone/update the Sedona fork used by Cargo.toml.
ref-sedona:
    @just _ref-clone-or-update sedona-db "{{ref_sedona_url}}" "{{ref_sedona_branch}}"

# Clone/update upstream SedonaDB for comparison/rebase context.
ref-sedona-upstream:
    @just _ref-clone-or-update sedona-db-upstream "{{ref_sedona_upstream_url}}" "{{ref_sedona_upstream_branch}}"

# Clone/update the datafusion-ducklake fork used by Cargo.toml.
ref-ducklake:
    @just _ref-clone-or-update datafusion-ducklake "{{ref_ducklake_url}}" "{{ref_ducklake_branch}}"

# Clone/update upstream datafusion-ducklake for comparison/rebase context.
ref-ducklake-upstream:
    @just _ref-clone-or-update datafusion-ducklake-upstream "{{ref_ducklake_upstream_url}}" "{{ref_ducklake_upstream_branch}}"

# Clone/update Martin tile server source.
ref-martin:
    @just _ref-clone-or-update martin "{{ref_martin_url}}" "{{ref_martin_branch}}"

# Clone/update QGIS source matching the default QGIS LTR probe image.
ref-qgis:
    @just _ref-clone-or-update qgis "{{ref_qgis_url}}" "{{ref_qgis_branch}}"

# Clone/update GeoServer source matching the default GeoServer probe image.
ref-geoserver:
    @just _ref-clone-or-update geoserver "{{ref_geoserver_url}}" "{{ref_geoserver_branch}}"

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

# Clone/update pg_ducklake source for v0.1/PG-catalog production comparison.
ref-pg-ducklake:
    @just _ref-clone-or-update pg-ducklake "{{ref_pg_ducklake_url}}" "{{ref_pg_ducklake_branch}}"

# Clone/update PostGIS source/regression tests.
ref-postgis:
    @just _ref-clone-or-update postgis "{{ref_postgis_url}}" "{{ref_postgis_branch}}"

# Clone/update SQLite source (DuckLake dev catalog backend and SQL reference).
ref-sqlite:
    @just _ref-clone-or-update sqlite "{{ref_sqlite_url}}" "{{ref_sqlite_branch}}"

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

# Faster local regression loop: only QuackGIS's non-ignored integration gates.
test-fast:
    cargo test -p quackgis-server --lib --test ducklake_persistence --test layoutbench_sf0 --test martin_compat --test postgis_regress --test wire_spatial

# Run the starter curated PostGIS function regress subset and print pass-rate evidence.
postgis-regress:
    cargo test -p quackgis-server --test postgis_regress -- --nocapture

# Run the deterministic LayoutBench sf0 oracle for spatial-layout work.
layoutbench-sf0:
    cargo test -p quackgis-server --test layoutbench_sf0 -- --nocapture

# Seed and run LayoutBench against an already-running local server.
layoutbench-local scale="sf0" query_iters="3" ingest_order="generated" load_method="insert" compact="false":
    @extra=""; \
    if [ "{{compact}}" = "true" ]; then extra="--compact-and-rerun"; fi; \
    cargo run -p quackgis-server --example layoutbench -- --host {{host}} --port {{port}} --scale {{scale}} --query-iters {{query_iters}} --ingest-order {{ingest_order}} --load-method {{load_method}} $extra

# CI/local smoke for the LayoutBench runner: start a temporary server, run sf0, and exit.
layoutbench-local-smoke:
    @set -eu; \
    rm -rf .tmp/layoutbench-smoke; \
    mkdir -p .tmp/layoutbench-smoke/data; \
    log=.tmp/layoutbench-smoke/quackgis-server.log; \
    bench_log=.tmp/layoutbench-smoke/layoutbench.log; \
    QUACKGIS_CATALOG_PATH=.tmp/layoutbench-smoke/quackgis.db QUACKGIS_DATA_PATH=.tmp/layoutbench-smoke/data cargo run -p quackgis-server -- --host {{smoke_host}} --port {{smoke_port}} > "$log" 2>&1 & \
    server_pid=$!; \
    trap 'kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true' EXIT INT TERM; \
    python3 scripts/wait_for_tcp.py {{smoke_host}} {{smoke_port}} "$server_pid" "$log"; \
    if ! cargo run -p quackgis-server --example layoutbench -- --host {{smoke_host}} --port {{smoke_port}} --scale sf0 --query-iters 1 > "$bench_log" 2>&1; then \
        python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"), end="")' "$bench_log"; \
        exit 1; \
    fi; \
    python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"); print(text, end=""); sys.exit(0 if "layoutbench_query label=aerial" in text and "layoutbench_pruning label=aerial" in text and "layoutbench_scan label=aerial" in text else 1)' "$bench_log"; \
    kill "$server_pid" 2>/dev/null || true; \
    wait "$server_pid" 2>/dev/null || true; \
    trap - EXIT INT TERM

# Run nextest when installed by mise.
nextest:
    cargo nextest run --workspace

# Full local verification gate.
check: fmt-check clippy test

# Faster local verification gate for edit/probe triage.
check-fast: fmt-check clippy test-fast

# Run the same fast gate used by GitHub Actions CI.
ci: check-fast smoke-local-demo preview-smoke probe-static-check runtime-static-check

# Run the dev QuackGIS server on QUACKGIS_HOST/QUACKGIS_PORT.
server:
    mkdir -p "$(dirname '{{catalog}}')" "{{data}}"
    cargo run -p quackgis-server -- --host {{host}} --port {{port}} --catalog-path "{{catalog}}" --data-path "{{data}}"

# Connect with psql to a running dev server.
psql:
    psql -h {{host}} -p {{port}} -U postgres -d quackgis

# Seed stable demo layers in an already-running local server.
seed-local-demo:
    cargo run -p quackgis-server --example seed_demo -- --host {{host}} --port {{port}}

# CI/local smoke: start a temporary server, seed stable demo layers, verify, and exit.
smoke-local-demo:
    @set -eu; \
    rm -rf .tmp/demo-smoke; \
    mkdir -p .tmp/demo-smoke/data; \
    log=.tmp/demo-smoke/quackgis-server.log; \
    seed_log=.tmp/demo-smoke/seed.log; \
    QUACKGIS_CATALOG_PATH=.tmp/demo-smoke/quackgis.db QUACKGIS_DATA_PATH=.tmp/demo-smoke/data cargo run -p quackgis-server -- --host {{smoke_host}} --port {{smoke_port}} > "$log" 2>&1 & \
    server_pid=$!; \
    trap 'kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true' EXIT INT TERM; \
    python3 scripts/wait_for_tcp.py {{smoke_host}} {{smoke_port}} "$server_pid" "$log"; \
    if ! QUACKGIS_HOST={{smoke_host}} QUACKGIS_PORT={{smoke_port}} cargo run -p quackgis-server --example seed_demo -- --host {{smoke_host}} --port {{smoke_port}} > "$seed_log" 2>&1; then \
        python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"), end="")' "$seed_log"; \
        exit 1; \
    fi; \
    python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"); print(text, end=""); sys.exit(0 if "demo_ok True" in text else 1)' "$seed_log"; \
    kill "$server_pid" 2>/dev/null || true; \
    wait "$server_pid" 2>/dev/null || true; \
    trap - EXIT INT TERM

# Developer-preview acceptance smoke: start a temp server and exercise CREATE, COPY, query, compact.
preview-smoke:
    @set -eu; \
    rm -rf .tmp/preview-smoke; \
    mkdir -p .tmp/preview-smoke/data; \
    log=.tmp/preview-smoke/quackgis-server.log; \
    preview_log=.tmp/preview-smoke/preview.log; \
    QUACKGIS_CATALOG_PATH=.tmp/preview-smoke/quackgis.db QUACKGIS_DATA_PATH=.tmp/preview-smoke/data cargo run -p quackgis-server -- --host {{smoke_host}} --port {{smoke_port}} > "$log" 2>&1 & \
    server_pid=$!; \
    trap 'kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true' EXIT INT TERM; \
    python3 scripts/wait_for_tcp.py {{smoke_host}} {{smoke_port}} "$server_pid" "$log"; \
    if ! cargo run -p quackgis-server --example developer_preview -- --host {{smoke_host}} --port {{smoke_port}} > "$preview_log" 2>&1; then \
        python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"), end="")' "$preview_log"; \
        exit 1; \
    fi; \
    python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"); print(text, end=""); sys.exit(0 if "developer_preview_ok True" in text else 1)' "$preview_log"; \
    kill "$server_pid" 2>/dev/null || true; \
    wait "$server_pid" 2>/dev/null || true; \
    trap - EXIT INT TERM

# One-command host-local demo: start QuackGIS, seed stable layers, and keep it running.
demo-local:
    @set -eu; \
    rm -rf .tmp/demo; \
    mkdir -p .tmp/demo/data; \
    log=.tmp/demo/quackgis-server.log; \
    QUACKGIS_CATALOG_PATH=.tmp/demo/quackgis.db QUACKGIS_DATA_PATH=.tmp/demo/data cargo run -p quackgis-server -- --host {{host}} --port {{port}} > "$log" 2>&1 & \
    server_pid=$!; \
    trap 'kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true' EXIT INT TERM; \
    python3 scripts/wait_for_tcp.py {{host}} {{port}} "$server_pid" "$log"; \
    QUACKGIS_HOST={{host}} QUACKGIS_PORT={{port}} cargo run -p quackgis-server --example seed_demo -- --host {{host}} --port {{port}}; \
    printf '\nLocal demo is running on host={{host}} port={{port}}. Press Ctrl-C to stop.\n'; \
    status=0; \
    wait "$server_pid" || status=$?; \
    if [ "$status" -eq 130 ] || [ "$status" -eq 143 ]; then exit 0; fi; \
    exit "$status"

# Remove local dev DuckLake catalog/data.
clean-dev:
    rm -rf .tmp/dev

# Download the static-ish Martin musl binary into .tmp/bin.
install-martin:
    mkdir -p .tmp/bin
    curl -fsSL "https://github.com/maplibre/martin/releases/download/v{{martin_version}}/martin-x86_64-unknown-linux-musl.tar.gz" -o .tmp/martin.tar.gz
    tar -xzf .tmp/martin.tar.gz -C .tmp/bin martin
    chmod +x .tmp/bin/martin
    {{martin_bin}} --version

# Run QuackGIS's passing Martin SQL compatibility gate.
martin-sql:
    cargo test -p quackgis-server --test martin_compat -- --nocapture

# Run the real Martin binary E2E (ignored by default; requires MARTIN_BIN).
martin-e2e: install-martin
    MARTIN_BIN="$(pwd)/{{martin_bin}}" cargo test -p quackgis-server --test martin_real_e2e -- --ignored --nocapture

# Start Martin against an already-running QuackGIS server (auto-discovery path).
martin:
    {{martin_bin}} --listen-addresses {{host}}:{{martin_port}} --auto-bounds skip --default-srid 3857 "postgres://postgres@{{host}}:{{port}}/quackgis"

# Pull the configured QGIS image for client-trace work.
qgis-pull:
    {{container_engine}} pull {{qgis_image}}

# Open a shell in the configured QGIS image.
qgis-shell:
    {{container_engine}} run --rm -it --network host {{qgis_image}} bash

# Pull the configured GeoServer image.
geoserver-pull:
    {{container_engine}} pull {{geoserver_image}}

# Run GeoServer locally on http://127.0.0.1:8080/geoserver.
geoserver:
    {{container_engine}} run --rm -it --network host -e SKIP_DEMO_DATA=true -e GEOSERVER_ADMIN_PASSWORD=geoserver {{geoserver_image}}

# Check host Podman plus mise-pinned Kind/kubectl before running in-cluster probes.
kind-doctor:
    mise --version
    {{container_engine}} --version
    {{container_engine}} info >/dev/null
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind version
    kubectl version --client=true
    @printf "Podman/Kind tooling looks usable. Run 'just kind-up' to create/reuse the local cluster.\n"

# Create or reuse the local Kind cluster for in-cluster client probes.
kind-up:
    @if KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind get clusters | grep -Fxq "{{kind_cluster}}"; then \
        printf "Kind cluster {{kind_cluster}} already exists; reusing it.\n"; \
    else \
        KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind create cluster --name {{kind_cluster}} --config deploy/kind/cluster.yaml; \
    fi

# Show local Kind cluster and QuackGIS namespace state.
kind-status:
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind get clusters
    kubectl cluster-info --context kind-{{kind_cluster}}
    kubectl get nodes -o wide
    kubectl -n quackgis get pods,jobs,svc,statefulset,deploy -o wide || true

# One-command local cluster bootstrap/check for new shells and machines.
kind-ready: kind-doctor kind-up kind-status

# Build the QuackGIS development image for Kind using the host Cargo cache.
kind-build-image:
    cargo build -p quackgis-server --release
    rm -rf .tmp/kind/runtime
    mkdir -p .tmp/kind/runtime
    cp target/release/quackgis-server .tmp/kind/runtime/quackgis-server
    {{container_engine}} build -t {{quackgis_image}} -f deploy/Containerfile.runtime .tmp/kind/runtime

# Faster Kind image for probe loops: optimized enough, no release thin-LTO.
kind-build-image-fast:
    cargo build -p quackgis-server --profile probe
    rm -rf .tmp/kind/runtime
    mkdir -p .tmp/kind/runtime
    cp target/probe/quackgis-server .tmp/kind/runtime/quackgis-server
    {{container_engine}} build -t {{quackgis_image}} -f deploy/Containerfile.runtime .tmp/kind/runtime

# Build the QuackGIS development image entirely inside the container build.
kind-build-image-container:
    {{container_engine}} build -t {{quackgis_image}} -f deploy/Containerfile .

# Load the QuackGIS development image into the Kind cluster.
kind-load-image:
    mkdir -p .tmp/kind
    rm -f .tmp/kind/quackgis-image.tar
    {{container_engine}} save {{quackgis_image}} -o .tmp/kind/quackgis-image.tar
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind load image-archive .tmp/kind/quackgis-image.tar --name {{kind_cluster}}

# Deploy QuackGIS into Kind behind service quackgis.quackgis.svc.cluster.local:5434.
kind-deploy:
    kubectl apply -f deploy/kind/quackgis.yaml
    kubectl -n quackgis rollout restart statefulset/quackgis
    kubectl -n quackgis rollout status statefulset/quackgis --timeout=180s
    kubectl -n quackgis wait pod -l app=quackgis --for=condition=ready --timeout=180s

# Publish shared Kind probe scripts as a ConfigMap consumed by all probe Jobs.
kind-probe-scripts:
    kubectl create namespace quackgis --dry-run=client -o yaml | kubectl apply -f -
    kubectl -n quackgis create configmap quackgis-probe-scripts --from-file=deploy/kind/probes --dry-run=client -o yaml | kubectl apply -f -

# Static pre-Kind validation for probe scripts and Kubernetes manifests.
probe-static-check:
    mkdir -p .tmp/pycache
    PYTHONPYCACHEPREFIX=.tmp/pycache python3 -m py_compile scripts/probe_static_check.py scripts/runtime_static_check.py scripts/trend_metrics.py deploy/kind/render_compat_report.py deploy/kind/check_linkerd_injected.py deploy/kind/probes/*.py
    bash -n deploy/kind/probes/*.sh
    python3 scripts/probe_static_check.py deploy/kind

# Static guard that the maintained runtime image remains one native-free Rust binary.
runtime-static-check:
    python3 scripts/runtime_static_check.py deploy/Containerfile.runtime

# Flatten one or more compatibility/storage metrics artifacts for trend analysis.
metrics-trend path=".tmp/compatibility" format="csv":
    python3 scripts/trend_metrics.py --format {{format}} "{{path}}"

# Build, load, and deploy QuackGIS into Kind.
kind-refresh: kind-build-image kind-load-image kind-deploy

# Faster build, load, and deploy loop for client-probe triage.
kind-refresh-fast: kind-build-image-fast kind-load-image kind-deploy

# Build, deploy, and run the maintained Kind client compatibility suite.
kind-compatibility:
    just kind-refresh-fast
    just kind-probes

# Install or upgrade Linkerd in Kind using Helm and repo-local ephemeral certs.
kind-linkerd-up: kind-ready
    mkdir -p .tmp/linkerd
    @if [ ! -s .tmp/linkerd/ca.crt ] || [ ! -s .tmp/linkerd/ca.key ] || [ ! -s .tmp/linkerd/issuer.crt ] || [ ! -s .tmp/linkerd/issuer.key ]; then \
        openssl ecparam -name prime256v1 -genkey -noout -out .tmp/linkerd/ca.key; \
        openssl req -x509 -new -key .tmp/linkerd/ca.key -sha256 -days 365 -out .tmp/linkerd/ca.crt -subj "/CN=root.linkerd.cluster.local" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"; \
        openssl ecparam -name prime256v1 -genkey -noout -out .tmp/linkerd/issuer.key; \
        openssl req -new -key .tmp/linkerd/issuer.key -out .tmp/linkerd/issuer.csr -subj "/CN=identity.linkerd.cluster.local"; \
        printf "basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n" > .tmp/linkerd/issuer-ext.cnf; \
        openssl x509 -req -in .tmp/linkerd/issuer.csr -CA .tmp/linkerd/ca.crt -CAkey .tmp/linkerd/ca.key -CAcreateserial -out .tmp/linkerd/issuer.crt -days 365 -sha256 -extfile .tmp/linkerd/issuer-ext.cnf; \
    fi
    helm repo add linkerd https://helm.linkerd.io/stable --force-update
    helm repo update linkerd
    kubectl create namespace linkerd --dry-run=client -o yaml | kubectl apply -f -
    helm upgrade --install linkerd-crds linkerd/linkerd-crds -n linkerd --wait --timeout 5m
    helm upgrade --install linkerd-control-plane linkerd/linkerd-control-plane -n linkerd --set-file identityTrustAnchorsPEM=.tmp/linkerd/ca.crt --set-file identity.issuer.tls.crtPEM=.tmp/linkerd/issuer.crt --set-file identity.issuer.tls.keyPEM=.tmp/linkerd/issuer.key --set proxyInit.iptablesMode={{linkerd_iptables_mode}} --wait --timeout 5m
    kubectl -n linkerd wait deployment --all --for=condition=Available --timeout=300s

# Deploy the lake profile: PostgreSQL DuckLake catalog + s3s-fs local S3.
kind-lake-deploy:
    kubectl create namespace quackgis --dry-run=client -o yaml | kubectl apply -f -
    kubectl apply -f deploy/kind/lake.yaml
    kubectl -n quackgis rollout status deployment/pg --timeout=180s
    kubectl -n quackgis rollout status deployment/s3 --timeout=180s
    kubectl -n quackgis wait pod -l app=pg --for=condition=ready --timeout=180s
    kubectl -n quackgis wait pod -l app=s3 --for=condition=ready --timeout=180s
    kubectl -n quackgis rollout restart deployment/lake
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s

# Build, load, and deploy the lake profile into Kind.
kind-lake-refresh: kind-build-image-fast kind-load-image kind-lake-deploy

# Run the lake storage smoke in Kind against PostgreSQL catalog + s3s-fs object storage.
kind-lake-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis delete job lake-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/lake-probe.yaml
    kubectl -n quackgis wait job/lake-probe --for=condition=complete --timeout=240s || (kubectl -n quackgis logs deployment/lake --tail=200 || true; kubectl -n quackgis logs job/lake-probe || true; false)
    kubectl -n quackgis logs job/lake-probe

# Run concurrent storage probes while QuackGIS is scaled to two pods.
kind-lake-multipod-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job lake-multipod --ignore-not-found=true
    kubectl apply -f deploy/kind/lake-multipod-probe.yaml
    kubectl -n quackgis wait job/lake-multipod --for=condition=complete --timeout=300s || (kubectl -n quackgis get pods -l app=lake -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs -l job-name=lake-multipod --all-containers=true --prefix=true || true; false)
    kubectl -n quackgis logs -l job-name=lake-multipod --all-containers=true --prefix=true

# Prove the Kubernetes Service distributes fresh pgwire TCP connections across pods.
kind-lb-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job lb-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/lb-probe.yaml
    kubectl -n quackgis wait job/lb-probe --for=condition=complete --timeout=240s || (kubectl -n quackgis get pods -l app=lake -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/lb-probe || true; false)
    kubectl -n quackgis logs job/lb-probe

# Deploy an injected client pod used for Linkerd mTLS probes without Job sidecar hangs.
kind-mesh-client-deploy: kind-probe-scripts
    kubectl apply -f deploy/kind/mesh-client.yaml
    kubectl -n quackgis rollout restart deployment/mesh-client
    kubectl -n quackgis rollout status deployment/mesh-client --timeout=180s
    kubectl -n quackgis wait pod -l app=mesh-client --for=condition=ready --timeout=180s

# Run lake storage and load-balancing probes through Linkerd-injected pods.
kind-mtls-smoke: kind-linkerd-up kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis rollout restart deployment/pg deployment/s3 deployment/lake
    kubectl -n quackgis rollout status deployment/pg --timeout=180s
    kubectl -n quackgis rollout status deployment/s3 --timeout=180s
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    just kind-mesh-client-deploy
    kubectl -n quackgis get pods -l 'app in (lake,pg,s3,mesh-client)' -o json | python3 deploy/kind/check_linkerd_injected.py lake pg s3 mesh-client
    kubectl -n quackgis exec deployment/mesh-client -c mesh-client -- env PYTHONPATH=/opt/quackgis-probes QUACKGIS_HOST=lake.quackgis.svc.cluster.local QUACKGIS_PORT=5434 LB_CONNECTIONS=40 LB_MIN_INSTANCES=2 python3 /opt/quackgis-probes/mtls_probe.py

# Run LayoutBench sf0 through the lake PostgreSQL catalog + S3 storage profile.
kind-lake-layoutbench-smoke: kind-ready
    @set -eu; \
    just kind-lake-refresh; \
    rm -rf .tmp/layoutbench-lake; \
    mkdir -p .tmp/layoutbench-lake; \
    log=.tmp/layoutbench-lake/port-forward.log; \
    bench_log=.tmp/layoutbench-lake/layoutbench.log; \
    kubectl -n quackgis port-forward service/lake {{smoke_port}}:5434 > "$log" 2>&1 & \
    pf_pid=$!; \
    trap 'kill "$pf_pid" 2>/dev/null || true; wait "$pf_pid" 2>/dev/null || true' EXIT INT TERM; \
    python3 scripts/wait_for_tcp.py {{smoke_host}} {{smoke_port}} "$pf_pid" "$log"; \
    if ! cargo run -p quackgis-server --example layoutbench -- --host {{smoke_host}} --port {{smoke_port}} --scale sf0 --query-iters 1 --prefix lake_layoutbench --load-method copy --compact-and-rerun > "$bench_log" 2>&1; then \
        python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"), end="")' "$bench_log"; \
        exit 1; \
    fi; \
    python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"); print(text, end=""); required = ["layoutbench_seed", "layoutbench_pruning label=aerial", "layoutbench_query label=aerial phase=before_compact", "layoutbench_scan label=aerial phase=after_compact", "layoutbench_compact"]; sys.exit(0 if all(item in text for item in required) else 1)' "$bench_log"; \
    kill "$pf_pid" 2>/dev/null || true; \
    wait "$pf_pid" 2>/dev/null || true; \
    trap - EXIT INT TERM

# Run an in-cluster concurrent read workload against the lake PostgreSQL/S3 profile.
kind-read-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job read-seed read-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/read-seed.yaml
    kubectl -n quackgis wait job/read-seed --for=condition=complete --timeout=600s || (kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/read-seed || true; false)
    kubectl -n quackgis logs job/read-seed
    kubectl apply -f deploy/kind/read-probe.yaml
    kubectl -n quackgis wait job/read-probe --for=condition=complete --timeout=600s || (kubectl -n quackgis get pods -l app=lake -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/read-probe || true; false)
    kubectl -n quackgis logs job/read-probe

# Run the high-concurrency parallel-reader gate against the lake PostgreSQL/S3 profile.
kind-qps-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis set env deployment/lake QUACKGIS_SHARED_CATALOG_REFRESH_MS={{qps_shared_catalog_refresh_ms}} QUACKGIS_TARGET_PARTITIONS- QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS-
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis scale deployment/lake --replicas=3
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job qps-seed qps-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/qps-seed.yaml
    kubectl -n quackgis wait job/qps-seed --for=condition=complete --timeout=600s || (kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/qps-seed || true; false)
    kubectl -n quackgis logs job/qps-seed
    kubectl apply -f deploy/kind/qps-probe.yaml
    kubectl -n quackgis wait job/qps-probe --for=condition=complete --timeout=600s || (kubectl -n quackgis get pods -l app=lake -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/qps-probe || true; false)
    kubectl -n quackgis logs job/qps-probe

# Run the high-concurrency reader gate from a Linkerd-injected client and assert proxy TCP/TLS metrics.
kind-qps-mtls-smoke: kind-linkerd-up kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis set env deployment/lake QUACKGIS_SHARED_CATALOG_REFRESH_MS={{qps_shared_catalog_refresh_ms}} QUACKGIS_TARGET_PARTITIONS- QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS-
    kubectl -n quackgis rollout restart deployment/pg deployment/s3 deployment/lake
    kubectl -n quackgis rollout status deployment/pg --timeout=180s
    kubectl -n quackgis rollout status deployment/s3 --timeout=180s
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis scale deployment/lake --replicas=3
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    just kind-mesh-client-deploy
    kubectl -n quackgis get pods -l 'app in (lake,pg,s3,mesh-client)' -o json | python3 deploy/kind/check_linkerd_injected.py lake pg s3 mesh-client
    kubectl -n quackgis exec deployment/mesh-client -c mesh-client -- env PYTHONPATH=/opt/quackgis-probes QUACKGIS_HOST=lake.quackgis.svc.cluster.local QUACKGIS_PORT=5434 QPS_FACTOR=50 QPS_WORKERS=16 QPS_QUERIES=240 QPS_MIN_INSTANCES=2 QPS_MIN_QPS=1.0 QPS_TABLE=qps_aerial QPS_MODE=seed python3 /opt/quackgis-probes/qps_probe.py
    kubectl -n quackgis exec deployment/mesh-client -c mesh-client -- env PYTHONPATH=/opt/quackgis-probes QUACKGIS_HOST=lake.quackgis.svc.cluster.local QUACKGIS_PORT=5434 QPS_FACTOR=50 QPS_WORKERS=16 QPS_QUERIES=240 QPS_MIN_INSTANCES=2 QPS_MIN_QPS=1.0 QPS_TABLE=qps_aerial QPS_MODE=probe QPS_LINKERD_METRICS_URL=http://127.0.0.1:4191/metrics QPS_REQUIRE_LINKERD=true python3 /opt/quackgis-probes/qps_probe.py

# Guard the opt-in deep QPS run against accidental disk-budget overruns.
qps-deep-disk-guard:
    python3 -c 'import shutil, sys; factor=int(sys.argv[1]); budget_gib=int(sys.argv[2]); rows=factor*108; estimate=rows*1024; budget=budget_gib*1024**3; free=shutil.disk_usage(".").free; print(f"qps_deep_disk rows={rows} estimated_bytes={estimate} budget_bytes={budget} free_bytes={free}"); sys.exit(1 if estimate > budget or free < estimate * 2 else 0)' {{qps_deep_factor}} {{qps_deep_disk_budget_gib}}

# Run a deeper Linkerd-observed QPS gate; tune QPS_DEEP_* env vars up to the disk budget.
kind-qps-deep-smoke: kind-linkerd-up kind-probe-scripts qps-deep-disk-guard
    just kind-lake-refresh
    kubectl -n quackgis set env deployment/lake QUACKGIS_SHARED_CATALOG_REFRESH_MS={{qps_shared_catalog_refresh_ms}} QUACKGIS_TARGET_PARTITIONS- QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS-
    kubectl -n quackgis rollout restart deployment/pg deployment/s3 deployment/lake
    kubectl -n quackgis rollout status deployment/pg --timeout=180s
    kubectl -n quackgis rollout status deployment/s3 --timeout=180s
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis scale deployment/lake --replicas={{qps_deep_replicas}}
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    just kind-mesh-client-deploy
    kubectl -n quackgis get pods -l 'app in (lake,pg,s3,mesh-client)' -o json | python3 deploy/kind/check_linkerd_injected.py lake pg s3 mesh-client
    kubectl -n quackgis exec deployment/mesh-client -c mesh-client -- env PYTHONPATH=/opt/quackgis-probes QUACKGIS_HOST=lake.quackgis.svc.cluster.local QUACKGIS_PORT=5434 QPS_FACTOR={{qps_deep_factor}} QPS_WORKERS={{qps_deep_workers}} QPS_QUERIES={{qps_deep_queries}} QPS_MIN_INSTANCES={{qps_deep_min_instances}} QPS_MIN_QPS={{qps_deep_min_qps}} QPS_TABLE=qps_deep_aerial QPS_MODE=seed python3 /opt/quackgis-probes/qps_probe.py
    kubectl -n quackgis exec deployment/mesh-client -c mesh-client -- env PYTHONPATH=/opt/quackgis-probes QUACKGIS_HOST=lake.quackgis.svc.cluster.local QUACKGIS_PORT=5434 QPS_FACTOR={{qps_deep_factor}} QPS_WORKERS={{qps_deep_workers}} QPS_QUERIES={{qps_deep_queries}} QPS_MIN_INSTANCES={{qps_deep_min_instances}} QPS_MIN_QPS={{qps_deep_min_qps}} QPS_TABLE=qps_deep_aerial QPS_MODE=probe QPS_LINKERD_METRICS_URL=http://127.0.0.1:4191/metrics QPS_REQUIRE_LINKERD=true python3 /opt/quackgis-probes/qps_probe.py

# Run concurrent write workloads plus deterministic snapshot conflict/retry evidence.
kind-write-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job write-setup write-workers write-verify --ignore-not-found=true
    kubectl apply -f deploy/kind/write-setup.yaml
    kubectl -n quackgis wait job/write-setup --for=condition=complete --timeout=240s || (kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/write-setup || true; false)
    kubectl -n quackgis logs job/write-setup
    kubectl apply -f deploy/kind/write-workers.yaml
    kubectl -n quackgis wait job/write-workers --for=condition=complete --timeout=600s || (kubectl -n quackgis get pods -l job-name=write-workers -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs -l job-name=write-workers --all-containers=true --prefix=true || true; false)
    kubectl -n quackgis logs -l job-name=write-workers --all-containers=true --prefix=true
    kubectl apply -f deploy/kind/write-verify.yaml
    kubectl -n quackgis wait job/write-verify --for=condition=complete --timeout=300s || (kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/write-verify || true; false)
    kubectl -n quackgis logs job/write-verify

# Run an OLAP fanout workload: grouped stats, pruning evidence, exact recheck.
kind-olap-smoke: kind-ready kind-probe-scripts
    just kind-lake-refresh
    kubectl -n quackgis scale deployment/lake --replicas=2
    kubectl -n quackgis rollout status deployment/lake --timeout=180s
    kubectl -n quackgis wait deployment/lake --for=condition=Available --timeout=180s
    kubectl -n quackgis delete job olap-seed olap-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/olap-seed.yaml
    kubectl -n quackgis wait job/olap-seed --for=condition=complete --timeout=600s || (kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/olap-seed || true; false)
    kubectl -n quackgis logs job/olap-seed
    kubectl apply -f deploy/kind/olap-probe.yaml
    kubectl -n quackgis wait job/olap-probe --for=condition=complete --timeout=600s || (kubectl -n quackgis get pods -l app=lake -o wide || true; kubectl -n quackgis logs deployment/lake --all-containers=true --tail=200 || true; kubectl -n quackgis logs job/olap-probe || true; false)
    kubectl -n quackgis logs job/olap-probe

# Run Alpha scaled-storage gates: storage, multi-pod, writer conflict/retry, QPS, and OLAP.
kind-alpha-smoke: kind-lake-smoke kind-lake-multipod-smoke kind-write-smoke kind-qps-smoke kind-olap-smoke
    @printf "kind-alpha-smoke complete\n"

# Run all maintained in-cluster client probes in one Kubernetes wait.
kind-probes: kind-probe-scripts
    kubectl -n quackgis delete job qgis-probe qgis-edit-probe ogr-probe geoserver-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/qgis-probe.yaml -f deploy/kind/qgis-edit-probe.yaml -f deploy/kind/ogr-probe.yaml -f deploy/kind/geoserver-probe.yaml
    kubectl -n quackgis wait job/qgis-probe job/qgis-edit-probe job/ogr-probe job/geoserver-probe --for=condition=complete --timeout=600s || (kubectl -n quackgis logs job/qgis-probe || true; kubectl -n quackgis logs job/qgis-edit-probe || true; kubectl -n quackgis logs job/ogr-probe || true; kubectl -n quackgis logs job/geoserver-probe || true; false)
    kubectl -n quackgis logs job/qgis-probe
    kubectl -n quackgis logs job/qgis-edit-probe
    kubectl -n quackgis logs job/ogr-probe
    kubectl -n quackgis logs job/geoserver-probe

# Run the headless QGIS client probe as an in-cluster Job.
kind-qgis-probe: kind-probe-scripts
    kubectl -n quackgis delete job qgis-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/qgis-probe.yaml
    kubectl -n quackgis wait job/qgis-probe --for=condition=complete --timeout=180s || (kubectl -n quackgis logs job/qgis-probe; false)
    kubectl -n quackgis logs job/qgis-probe

# Run the headless QGIS edit/save probe as an in-cluster Job.
kind-qgis-edit-probe: kind-probe-scripts
    kubectl -n quackgis delete job qgis-edit-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/qgis-edit-probe.yaml
    kubectl -n quackgis wait job/qgis-edit-probe --for=condition=complete --timeout=240s || (kubectl -n quackgis logs job/qgis-edit-probe; false)
    kubectl -n quackgis logs job/qgis-edit-probe

# Run the GDAL/OGR PostgreSQL-driver load/read probe as an in-cluster Job.
kind-ogr-probe: kind-probe-scripts
    kubectl -n quackgis delete job ogr-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/ogr-probe.yaml
    kubectl -n quackgis wait job/ogr-probe --for=condition=complete --timeout=180s || (kubectl -n quackgis logs job/ogr-probe; false)
    kubectl -n quackgis logs job/ogr-probe

# Run the GeoServer PostGIS datastore/WFS/WMS/WFS-T probe as an in-cluster Job.
kind-geoserver-probe: kind-probe-scripts
    kubectl -n quackgis delete job geoserver-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/geoserver-probe.yaml
    kubectl -n quackgis wait job/geoserver-probe --for=condition=complete --timeout=600s || (kubectl -n quackgis logs job/geoserver-probe; false)
    kubectl -n quackgis logs job/geoserver-probe

# Seed stable demo layers and print client connection hints.
seed-kind-demo: kind-probe-scripts
    kubectl -n quackgis delete job quackgis-demo --ignore-not-found=true
    kubectl apply -f deploy/kind/demo.yaml
    kubectl -n quackgis wait job/quackgis-demo --for=condition=complete --timeout=180s || (kubectl -n quackgis logs job/quackgis-demo; false)
    kubectl -n quackgis logs job/quackgis-demo

# One-command local demo: deploy, seed stable layers, and print client hints.
demo-kind: kind-ready kind-refresh-fast seed-kind-demo

# Start the opt-in real-OSM PostGIS reference deployment used by parity probes.
kind-postgis-osm-up:
    kubectl apply -f deploy/kind/postgis-osm.yaml
    kubectl -n quackgis set image deployment/postgis-osm postgis={{postgis_image}}
    kubectl -n quackgis rollout status deployment/postgis-osm --timeout=180s
    kubectl -n quackgis wait pod -l app=postgis-osm --for=condition=ready --timeout=180s

# Stop the opt-in real-OSM PostGIS reference deployment.
kind-postgis-osm-down:
    kubectl -n quackgis delete job osm-postgis-parity --ignore-not-found=true
    kubectl -n quackgis delete configmap osm-parity-config --ignore-not-found=true
    kubectl -n quackgis delete deployment postgis-osm --ignore-not-found=true
    kubectl -n quackgis delete service postgis-osm --ignore-not-found=true

# Run the opt-in real OSM PostGIS -> QuackGIS copy/read parity probe.
kind-osm-postgis-parity: kind-postgis-osm-up
    kubectl -n quackgis delete job osm-postgis-parity --ignore-not-found=true
    kubectl -n quackgis delete configmap osm-parity-config --ignore-not-found=true
    kubectl -n quackgis create configmap osm-parity-config --from-literal=OSM_EXTRACT_URL="{{osm_extract_url}}" --from-literal=OSM_POINT_LIMIT="{{osm_point_limit}}" --from-literal=OSM_LINE_LIMIT="{{osm_line_limit}}" --from-literal=OSM_POLYGON_LIMIT="{{osm_polygon_limit}}"
    kubectl apply -f deploy/kind/osm-postgis-parity-probe.yaml
    kubectl -n quackgis wait job/osm-postgis-parity --for=condition=complete --timeout=900s || (kubectl -n quackgis logs job/osm-postgis-parity; false)
    kubectl -n quackgis logs job/osm-postgis-parity

# Collect Kind compatibility logs for CI/local artifacts.
kind-compat-report:
    mkdir -p .tmp/compatibility
    kubectl -n quackgis get pods,jobs,svc,deploy,statefulset -o wide > .tmp/compatibility/kubernetes.txt 2>&1 || true
    kubectl -n quackgis logs statefulset/quackgis --tail=-1 > .tmp/compatibility/quackgis.log 2>&1 || true
    @for job in qgis-probe qgis-edit-probe ogr-probe geoserver-probe osm-postgis-parity quackgis-demo lake-probe lake-multipod lb-probe read-seed read-probe qps-seed qps-probe write-setup write-workers write-verify olap-seed olap-probe; do \
        kubectl -n quackgis logs "job/${job}" --tail=-1 > ".tmp/compatibility/${job}.log" 2>&1 || true; \
    done
    kubectl -n quackgis logs deployment/mesh-client --all-containers=true --tail=-1 > .tmp/compatibility/mesh-client.log 2>&1 || true
    kubectl -n linkerd logs deployment/linkerd-identity --all-containers=true --tail=-1 > .tmp/compatibility/linkerd-identity.log 2>&1 || true
    python3 deploy/kind/render_compat_report.py .tmp/compatibility

# Show QuackGIS and client-probe logs from Kind.
kind-logs:
    kubectl -n quackgis logs statefulset/quackgis --tail=200

# Delete the local KinD cluster.
kind-down:
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind delete cluster --name {{kind_cluster}}
