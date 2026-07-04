#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Build the QuackGIS facade container image.
#
# Usage:
#   ./container/build.sh                 # build quackgis:dev
#   ./container/build.sh -t mytag        # build with a custom tag
#
# Environment:
#   QUACKGIS_IMAGE  Base image name (default: quackgis)
#   QUACKGIS_TAG    Image tag (default: dev)
set -euo pipefail

cd "$(dirname "$0")/.."

IMAGE="${QUACKGIS_IMAGE:-quackgis}"
TAG="${QUACKGIS_TAG:-dev}"

# Allow override via -t flag
while getopts "t:" opt; do
    case $opt in
        t) TAG="$OPTARG" ;;
        *) echo "Usage: $0 [-t tag]"; exit 1 ;;
    esac
done

CONTAINER_CMD="${CONTAINER_CMD:-docker}"

echo ">> Building ${IMAGE}:${TAG}"
exec "${CONTAINER_CMD}" build \
    -f container/Dockerfile \
    -t "${IMAGE}:${TAG}" \
    .
