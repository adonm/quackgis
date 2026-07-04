#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS release script.
#
# Orchestrates the full release pipeline: engine checks, image build, facade
# tests, SBOM generation, and git tagging.
#
# Usage:
#   ./deploy/release.sh 0.1.0             # release v0.1.0
#   ./deploy/release.sh 0.1.0 --skip-engine-tests  # skip engine phase
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:?Usage: $0 <version> [flags]}"
SKIP_ENGINE=false

for arg in "${@:2}"; do
    case "$arg" in
        --skip-engine-tests) SKIP_ENGINE=true ;;
    esac
done

TAG="quackgis:${VERSION}"
FAIL=0

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  QuackGIS release pipeline — v${VERSION}                          ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo

# ── 1. Engine checks ─────────────────────────────────────────────────────────

if ! $SKIP_ENGINE; then
    echo "── 1. Engine checks ─────────────────────────────────────────"
    if ./ci/all-checks.sh; then
        echo "✓ Engine checks passed"
    else
        echo "✗ Engine checks FAILED"
        FAIL=1
    fi
    echo
else
    echo "── 1. Engine checks skipped (--skip-engine-tests) ──────────"
    echo
fi

# ── 2. Build image ───────────────────────────────────────────────────────────

echo "── 2. Build image ───────────────────────────────────────────"
if QUACKGIS_TAG="$VERSION" ./container/build.sh -t "$VERSION"; then
    echo "✓ Image built: ${TAG}"
else
    echo "✗ Image build FAILED"
    FAIL=1
fi
echo

# ── 3. Generate version manifest ────────────────────────────────────────────

echo "── 3. Version manifest ──────────────────────────────────────"
MANIFEST_FILE="build/release/quackgis-${VERSION}-manifest.txt"
mkdir -p build/release

{
    echo "QuackGIS version manifest"
    echo "========================="
    echo "Version: ${VERSION}"
    echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Git commit: $(git rev-parse HEAD)"
    echo "Git branch: $(git rev-parse --abbrev-ref HEAD)"
    echo
    echo "Image: ${TAG}"
    echo "Image size: $(docker images "${TAG}" --format '{{.Size}}' | head -1)"
    echo "Layers: $(docker inspect "${TAG}" --format '{{range .RootFS.Layers}}{{println .}}{{end}}' | wc -l)"
    echo
    echo "OCI labels:"
    docker inspect "${TAG}" --format '{{range $k, $v := .Config.Labels}}  {{$k}} = {{$v}}{{println}}{{end}}' 2>/dev/null || true
    echo
    echo "Bundled versions (from image):"
    docker run --rm --entrypoint sh "${TAG}" -c \
        'echo "  PostgreSQL: $(psql --version 2>/dev/null || echo unknown)";
         echo "  DuckDB: $(echo SELECT duckdb_version\; | psql -U postgres -d postgres -tA 2>/dev/null || echo unknown)";
         echo "  GEOS: $(geos-config --version 2>/dev/null || echo unknown)";
         echo "  PROJ: $(proj 2>&1 | head -1 2>/dev/null || echo unknown)";
         echo "  GDAL: $(gdal-config --version 2>/dev/null || echo unknown)"' \
        2>/dev/null || echo "  (inspection failed)"
} > "$MANIFEST_FILE"

cat "$MANIFEST_FILE"
echo

# ── 4. SBOM ──────────────────────────────────────────────────────────────────

echo "── 4. SBOM ──────────────────────────────────────────────────"
SBOM_FILE="build/release/quackgis-${VERSION}-sbom.spdx.json"

if command -v syft >/dev/null 2>&1; then
    syft "${TAG}" -o spdx-json > "$SBOM_FILE"
    echo "✓ SBOM: ${SBOM_FILE}"
else
    echo "⚠ syft not found — SBOM skipped (install: curl -sSf https://raw.githubusercontent.com/anchore/syft/main/install.sh | sh -s -- -b /usr/local/bin)"
fi
echo

# ── 5. Summary ───────────────────────────────────────────────────────────────

echo "╔══════════════════════════════════════════════════════════════╗"
if [ "$FAIL" -eq 0 ]; then
    echo "║  RELEASE v${VERSION} READY                                       ║"
    echo "║                                                             ║"
    echo "║  Next steps:                                                ║"
    echo "║  1. Review manifest: ${MANIFEST_FILE}   ║"
    echo "║  2. Run facade tests: ./container/run-all-tests.sh          ║"
    echo "║  3. Tag: git tag -a v${VERSION} -m 'Release v${VERSION}'             ║"
    echo "║  4. Push: git push origin v${VERSION}                           ║"
    echo "║  5. Push image: docker push ${TAG}                ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "║  RELEASE FAILED — see output above                          ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    exit 1
fi
