# SPDX-License-Identifier: Apache-2.0
set dotenv-load := true

host := env_var_or_default("QUACKGIS_HOST", "127.0.0.1")
port := env_var_or_default("QUACKGIS_PORT", "5434")
catalog := env_var_or_default("QUACKGIS_CATALOG_PATH", ".tmp/dev/quackgis.db")
data := env_var_or_default("QUACKGIS_DATA_PATH", ".tmp/dev/data")
martin_bin := env_var_or_default("MARTIN_BIN", ".tmp/bin/martin")
martin_version := env_var_or_default("MARTIN_VERSION", "1.11.0")
martin_port := env_var_or_default("MARTIN_PORT", "3000")
qgis_image := env_var_or_default("QGIS_IMAGE", "docker.io/qgis/qgis:release-3_40")
geoserver_image := env_var_or_default("GEOSERVER_IMAGE", "docker.io/kartoza/geoserver:2.26.2")
kind_cluster := env_var_or_default("KIND_CLUSTER", "quackgis")

default:
    just --list

# Install mise-managed tools and project-local helper binaries.
setup: install-martin
    mise install

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
    docker pull {{qgis_image}}

# Open a shell in the configured QGIS image.
qgis-shell:
    docker run --rm -it --network host {{qgis_image}} bash

# Pull the configured GeoServer image.
geoserver-pull:
    docker pull {{geoserver_image}}

# Run GeoServer locally on http://127.0.0.1:8080/geoserver.
geoserver:
    docker run --rm -it --network host -e GEOSERVER_ADMIN_PASSWORD=geoserver {{geoserver_image}}

# Create a local KinD cluster for deployment smoke tests.
kind-up:
    kind create cluster --name {{kind_cluster}}

# Delete the local KinD cluster.
kind-down:
    kind delete cluster --name {{kind_cluster}}
