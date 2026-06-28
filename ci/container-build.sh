#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Build the reproducible duckdb_sedona builder image (GDAL 3.13.1 + Rust).
# Usage: ./ci/container-build.sh
set -euo pipefail

cd "$(dirname "$0")/.."

IMG="${SEDONADB_BUILDER_IMG:-ghcr.io/osgeo/gdal:ubuntu-full-3.13.1-amd64}"
TAG="${SEDONADB_BUILDER_TAG:-sedonadb-builder:latest}"

echo ">> building ${TAG} from ${IMG}"
exec podman build \
    --build-arg RUST_VERSION=1.88.0 \
    -f ci/Containerfile \
    -t "${TAG}" \
    .
