#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Build + test the extension inside the reproducible builder container, with no
# dependency on the host's Linuxbrew. The workspace is bind-mounted so cargo
# uses the live source; target/ and the cargo registry are cached in named
# volumes across runs.
#
# Usage:
#   ./ci/container-build.sh          # one-time: build the image
#   ./ci/container-test.sh           # then: cargo test --lib + cargo build --release
set -euo pipefail

cd "$(dirname "$0")/.."

TAG="${SEDONADB_BUILDER_TAG:-sedonadb-builder:latest}"

# libclang ships under a versioned llvm dir (/usr/lib/llvm-NN/lib); resolve it
# at runtime so the gdal crate's bindgen finds libclang.so regardless of the
# apt-shipped llvm version.
LIBCLANG=$(podman run --rm "${TAG}" \
    bash -lc 'dirname "$(find /usr/lib/llvm* -name libclang.so 2>/dev/null | head -1)"')

echo ">> LIBCLANG_PATH=${LIBCLANG}"
echo ">> cargo test --lib"
podman run --rm \
    -v "${PWD}:/work:Z" \
    -v sedonadb-cargo-registry:/root/.cargo/registry:Z \
    -v sedonadb-cargo-git:/root/.cargo/git:Z \
    -e LIBCLANG_PATH="${LIBCLANG}" \
    -e PKG_CONFIG_PATH=/usr/local/lib/pkgconfig \
    -e LD_LIBRARY_PATH=/usr/local/lib \
    -w /work \
    "${TAG}" \
    cargo test --lib

echo ">> cargo build --release"
podman run --rm \
    -v "${PWD}:/work:Z" \
    -v sedonadb-cargo-registry:/root/.cargo/registry:Z \
    -v sedonadb-cargo-git:/root/.cargo/git:Z \
    -e LIBCLANG_PATH="${LIBCLANG}" \
    -e PKG_CONFIG_PATH=/usr/local/lib/pkgconfig \
    -e LD_LIBRARY_PATH=/usr/local/lib \
    -w /work \
    "${TAG}" \
    cargo build --release
