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
    cargo test -p quackgis-server --lib --test ducklake_persistence --test layoutbench_sf0 --test martin_compat --test wire_spatial

# Run the deterministic LayoutBench sf0 oracle for spatial-layout work.
layoutbench-sf0:
    cargo test -p quackgis-server --test layoutbench_sf0 -- --nocapture

# Run nextest when installed by mise.
nextest:
    cargo nextest run --workspace

# Full local verification gate.
check: fmt-check clippy test

# Faster local verification gate for edit/probe triage.
check-fast: fmt-check clippy test-fast

# Run the same fast gate used by GitHub Actions CI.
ci: check-fast smoke-local-demo

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

# Build, load, and deploy QuackGIS into Kind.
kind-refresh: kind-build-image kind-load-image kind-deploy

# Faster build, load, and deploy loop for client-probe triage.
kind-refresh-fast: kind-build-image-fast kind-load-image kind-deploy

# Build, deploy, and run the maintained Kind client compatibility suite.
kind-compatibility:
    just kind-refresh-fast
    just kind-probes

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
    @for job in qgis-probe qgis-edit-probe ogr-probe geoserver-probe osm-postgis-parity quackgis-demo; do \
        kubectl -n quackgis logs "job/${job}" --tail=-1 > ".tmp/compatibility/${job}.log" 2>&1 || true; \
    done
    python3 deploy/kind/render_compat_report.py .tmp/compatibility

# Show QuackGIS and client-probe logs from Kind.
kind-logs:
    kubectl -n quackgis logs statefulset/quackgis --tail=200

# Delete the local KinD cluster.
kind-down:
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind delete cluster --name {{kind_cluster}}
