#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-compose.sh"

server_options=$(
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U postgres \
        -d quackgis \
        -c "SELECT array_to_string(srvoptions, ',') FROM pg_foreign_server WHERE srvname = 'quack_worker'"
)
[ "$server_options" = "quack_host=127.0.0.1:9494,disable_ssl=true" ] || {
    echo "PostgreSQL is not attached to the loopback tunnel: $server_options" >&2
    exit 1
}

if "$compose" exec -T postgres bash -lc \
    'exec 3<>/dev/tcp/worker/9494' >/dev/null 2>&1; then
    echo "worker Quack unexpectedly accepts direct network connections" >&2
    exit 1
fi

restore_client() {
    "$compose" start iroh_client >/dev/null 2>&1 || true
}
trap restore_client EXIT INT TERM

"$compose" stop iroh_client >/dev/null
if "$compose" exec -T postgres psql \
    -XAt \
    -v ON_ERROR_STOP=1 \
    -U quackgis_reader \
    -d quackgis \
    -c "SELECT count(*) FROM public.features" >/dev/null 2>&1; then
    echo "remote query unexpectedly bypassed the stopped iroh tunnel" >&2
    exit 1
fi

"$compose" start iroh_client >/dev/null
attempt=0
until rows=$(
    "$compose" exec -T postgres psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U quackgis_reader \
        -d quackgis \
        -c "SELECT count(*) FROM public.features" 2>/dev/null
); do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        echo "remote query did not recover after restarting the iroh tunnel" >&2
        exit 1
    fi
    sleep 1
done

[ "$rows" = "3" ] || {
    echo "unexpected row count after iroh reconnect: $rows" >&2
    exit 1
}

trap - EXIT INT TERM
echo "QuackGIS iroh loopback isolation and fail-closed reconnect passed"
