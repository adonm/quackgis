#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose_file="$root/deploy/quackgis/compose.yaml"

if [ -n "${CONTAINER_ENGINE:-}" ]; then
    case "$CONTAINER_ENGINE" in
        docker) set -- docker compose -f "$compose_file" "$@" ;;
        podman) set -- podman compose -f "$compose_file" "$@" ;;
        *) echo "unsupported CONTAINER_ENGINE: $CONTAINER_ENGINE" >&2; exit 2 ;;
    esac
elif command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    set -- docker compose -f "$compose_file" "$@"
elif command -v podman >/dev/null 2>&1 && podman compose version >/dev/null 2>&1; then
    set -- podman compose -f "$compose_file" "$@"
else
    echo "Docker Compose or Podman with a Compose provider is required" >&2
    exit 2
fi

exec "$@"
