# SPDX-License-Identifier: Apache-2.0
set dotenv-load := true

host := env_var_or_default("QUACKGIS_HOST", "127.0.0.1")
port := env_var_or_default("QUACKGIS_PORT", "5434")
catalog := env_var_or_default("QUACKGIS_CATALOG_PATH", ".tmp/dev/quackgis.db")
data := env_var_or_default("QUACKGIS_DATA_PATH", ".tmp/dev/data")
martin_bin := env_var_or_default("MARTIN_BIN", ".tmp/bin/martin")
martin_version := env_var_or_default("MARTIN_VERSION", "1.11.0")
martin_port := env_var_or_default("MARTIN_PORT", "3000")
qgis_image := env_var_or_default("QGIS_IMAGE", "docker.io/qgis/qgis:ltr-questing")
geoserver_image := env_var_or_default("GEOSERVER_IMAGE", "docker.io/kartoza/geoserver:2.26.2")
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
setup: install-martin
    mise install

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
    LD_LIBRARY_PATH= cargo clippy --workspace --all-targets -- -D warnings

# Run all default tests.
test:
    LD_LIBRARY_PATH= cargo test --workspace

# Run nextest when installed by mise.
nextest:
    LD_LIBRARY_PATH= cargo nextest run --workspace

# Full local verification gate.
check: fmt-check clippy test

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

# Download the static-ish Martin musl binary into .tmp/bin.
install-martin:
    mkdir -p .tmp/bin
    curl -fsSL "https://github.com/maplibre/martin/releases/download/v{{martin_version}}/martin-x86_64-unknown-linux-musl.tar.gz" -o .tmp/martin.tar.gz
    tar -xzf .tmp/martin.tar.gz -C .tmp/bin martin
    chmod +x .tmp/bin/martin
    {{martin_bin}} --version

# Run QuackGIS's passing Martin SQL compatibility gate.
martin-sql:
    LD_LIBRARY_PATH= cargo test -p quackgis-server --test martin_compat -- --nocapture

# Run the real Martin binary E2E (ignored by default; requires MARTIN_BIN).
martin-e2e: install-martin
    MARTIN_BIN="$(pwd)/{{martin_bin}}" LD_LIBRARY_PATH= cargo test -p quackgis-server --test martin_real_e2e -- --ignored --nocapture

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
    {{container_engine}} run --rm -it --network host -e GEOSERVER_ADMIN_PASSWORD=geoserver {{geoserver_image}}

# Create a local Kind cluster for in-cluster client probes.
kind-up:
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind create cluster --name {{kind_cluster}} --config deploy/kind/cluster.yaml

# Build the QuackGIS development image for Kind using the host Cargo cache.
kind-build-image:
    cargo build -p quackgis-server --release
    rm -rf .tmp/kind/runtime
    mkdir -p .tmp/kind/runtime
    cp target/release/quackgis-server .tmp/kind/runtime/quackgis-server
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

# Build, load, and deploy QuackGIS into Kind.
kind-refresh: kind-build-image kind-load-image kind-deploy

# Run the headless QGIS client probe as an in-cluster Job.
kind-qgis-probe:
    kubectl -n quackgis delete job qgis-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/qgis-probe.yaml
    kubectl -n quackgis wait job/qgis-probe --for=condition=complete --timeout=180s || (kubectl -n quackgis logs job/qgis-probe; false)
    kubectl -n quackgis logs job/qgis-probe

# Run the GDAL/OGR PostgreSQL-driver read probe as an in-cluster Job.
kind-ogr-probe:
    kubectl -n quackgis delete job ogr-probe --ignore-not-found=true
    kubectl apply -f deploy/kind/ogr-probe.yaml
    kubectl -n quackgis wait job/ogr-probe --for=condition=complete --timeout=180s || (kubectl -n quackgis logs job/ogr-probe; false)
    kubectl -n quackgis logs job/ogr-probe

# Show QuackGIS and client-probe logs from Kind.
kind-logs:
    kubectl -n quackgis logs statefulset/quackgis --tail=200

# Delete the local KinD cluster.
kind-down:
    KIND_EXPERIMENTAL_PROVIDER={{container_engine}} kind delete cluster --name {{kind_cluster}}
