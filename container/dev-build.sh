#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Fast iteration build for QuackGIS.
#
# All compilation happens inside Docker with BuildKit cache mounts.
# First build: ~30 min (DuckDB + Rust). Subsequent: minutes (cached).
# Init script changes only: seconds.
#
# Usage:
#   ./container/dev-build.sh                # full build (all stages)
#   docker build -t quackgis:dev -f container/Dockerfile.dev .  # same thing
set -euo pipefail

cd "$(dirname "$0")/.."

echo ">> Building QuackGIS dev image (Dockerfile.dev)..."
echo ">> First build takes ~30 min (DuckDB compilation). Cached after."
echo

DOCKER_BUILDKIT=1 docker build -t quackgis:dev -f container/Dockerfile.dev .

echo
echo ">> Done. Test with:"
echo "   docker rm -f quackgis-test 2>/dev/null"
echo "   docker run -d --name quackgis-test -e POSTGRES_PASSWORD=quackgis -p 55432:5432 quackgis:dev"
echo "   docker exec quackgis-test psql -U postgres -d postgres -tAc \"SELECT st_astext(st_geomfromtext('POINT(1 2)'))\""
