#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Generate SBOM and report image size for QuackGIS release tracking.
#
# Requires: docker, and optionally syft (for SBOM).
#
# Usage:
#   ./deploy/image-report.sh [image:tag]
set -euo pipefail

IMAGE_TAG="${1:-quackgis:dev}"
CONTAINER_CMD="${CONTAINER_CMD:-docker}"

echo ">> Image: ${IMAGE_TAG}"
echo

# ── Image size ───────────────────────────────────────────────────────────────

SIZE=$("${CONTAINER_CMD}" images "${IMAGE_TAG}" --format '{{.Size}}' | head -1)
echo "Image size: ${SIZE}"

# Uncompressed size (sum of layers).
LAYERS=$("${CONTAINER_CMD}" inspect "${IMAGE_TAG}" \
    --format '{{range .RootFS.Layers}}{{println .}}{{end}}' | wc -l)
echo "Layer count: ${LAYERS}"

# ── SBOM (optional, requires syft) ──────────────────────────────────────────

if command -v syft >/dev/null 2>&1; then
    echo
    echo ">> Generating SBOM with syft..."
    syft "${IMAGE_TAG}" -o spdx-json > quackgis-sbom.spdx.json
    echo "   SBOM: quackgis-sbom.spdx.json"
else
    echo
    echo "⚠ syft not found — install with: curl -sSf https://raw.githubusercontent.com/anchore/syft/main/install.sh | sh -s -- -b /usr/local/bin"
    echo "  SBOM generation skipped."
fi

# ── OCI labels ───────────────────────────────────────────────────────────────

echo
echo ">> OCI labels:"
"${CONTAINER_CMD}" inspect "${IMAGE_TAG}" \
    --format '{{range $k, $v := .Config.Labels}}{{println $k "=" $v}}{{end}}' \
    2>/dev/null || echo "  (no labels found)"

echo
echo ">> Versions inside the image:"
"${CONTAINER_CMD}" run --rm --entrypoint sh "${IMAGE_TAG}" -c \
    'psql --version 2>/dev/null || true; \
     echo "duckdb: $(duckdb --version 2>/dev/null || echo unknown)"; \
     echo "geos: $(geos-config --version 2>/dev/null || echo unknown)"; \
     echo "proj: $(proj 2>&1 | head -1 || echo unknown)"; \
     echo "gdal: $(gdal-config --version 2>/dev/null || echo unknown)"' \
    2>/dev/null || echo "  (could not inspect)"
