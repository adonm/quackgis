#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
compose="$root/scripts/quackgis-multi-compose.sh"

query() {
    service=$1
    sql=$2
    "$compose" exec -T -e PGPASSWORD=quackgis-reader-dev "$service" psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -h 127.0.0.1 \
        -U quackgis_reader \
        -d quackgis \
        -c "$sql"
}

identity_sql="SELECT concat_ws('|', worker_id, catalog_path, snapshot_id, row_count, data_file_count, data_files) FROM remote.worker_identity_export"

attempt=0
until identity_a=$(query postgres_a "$identity_sql" 2>/dev/null) \
    && identity_b=$(query postgres_b "$identity_sql" 2>/dev/null); do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        echo "multi-worker QuackGIS edges did not become queryable" >&2
        exit 1
    fi
    sleep 2
done

case "$identity_a" in
    worker-a\|*) ;;
    *) echo "edge A reached the wrong worker: $identity_a" >&2; exit 1 ;;
esac
case "$identity_b" in
    worker-b\|*) ;;
    *) echo "edge B reached the wrong worker: $identity_b" >&2; exit 1 ;;
esac

shared_a=${identity_a#worker-a|}
shared_b=${identity_b#worker-b|}
[ "$shared_a" = "$shared_b" ] || {
    echo "workers do not report the same DuckLake snapshot/files" >&2
    echo "worker A: $identity_a" >&2
    echo "worker B: $identity_b" >&2
    exit 1
}

catalog_path=$(printf '%s\n' "$identity_a" | cut -d '|' -f 2)
snapshot_id=$(printf '%s\n' "$identity_a" | cut -d '|' -f 3)
row_count=$(printf '%s\n' "$identity_a" | cut -d '|' -f 4)
data_file_count=$(printf '%s\n' "$identity_a" | cut -d '|' -f 5)

[ "$catalog_path" = "/lake/catalog.ducklake" ] || {
    echo "unexpected shared DuckLake catalog: $catalog_path" >&2
    exit 1
}
case "$snapshot_id" in
    ''|*[!0-9]*) echo "invalid DuckLake snapshot id: $snapshot_id" >&2; exit 1 ;;
esac
[ "$snapshot_id" -gt 0 ] || {
    echo "DuckLake snapshot did not advance beyond initialization" >&2
    exit 1
}
[ "$row_count" = "3" ] || {
    echo "unexpected DuckLake row count: $row_count" >&2
    exit 1
}
case "$data_file_count" in
    ''|*[!0-9]*) echo "invalid DuckLake data-file count: $data_file_count" >&2; exit 1 ;;
esac
[ "$data_file_count" -gt 0 ] || {
    echo "DuckLake did not publish an external Parquet data file" >&2
    exit 1
}

for edge_worker in 'postgres_a worker_a' 'postgres_b worker_b'; do
    edge=${edge_worker%% *}
    worker=${edge_worker#* }
    if "$compose" exec -T "$edge" bash -lc \
        "exec 3<>/dev/tcp/$worker/9494" >/dev/null 2>&1; then
        echo "$edge can bypass iroh and reach $worker Quack directly" >&2
        exit 1
    fi
done

for worker in worker_a worker_b; do
    if "$compose" exec -T "$worker" /bin/sh -c \
        'touch /lake/quackgis-write-probe' >/dev/null 2>&1; then
        echo "$worker can write the shared DuckLake volume" >&2
        exit 1
    fi
done

signature_sql="SELECT count(*)::text || '|' || sum(id)::text || '|' || string_agg(name, ',' ORDER BY id) || '|' || ST_Extent(geom)::text FROM public.features"
expected_signature='3|6|west,east,south|BOX(-123.1 48.9,-122.9 49.25)'
for edge in postgres_a postgres_b; do
    signature=$(query "$edge" "$signature_sql")
    [ "$signature" = "$expected_signature" ] || {
        echo "$edge returned an unexpected shared dataset signature: $signature" >&2
        exit 1
    }

    viewport=$(query "$edge" "SELECT array_agg(id ORDER BY id) FROM public.features WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326)")
    [ "$viewport" = "{1}" ] || {
        echo "$edge returned an unexpected viewport: $viewport" >&2
        exit 1
    }

    plan=$(query "$edge" "EXPLAIN (VERBOSE, COSTS OFF) SELECT id FROM public.features WHERE geom && ST_MakeEnvelope(-123.2, 49.1, -123.0, 49.3, 4326)")
    for fragment in '"minx" <=' '"maxx" >=' '"miny" <=' '"maxy" >=' 'Filter:'; do
        printf '%s\n' "$plan" | grep -F "$fragment" >/dev/null || {
            echo "$edge plan is missing: $fragment" >&2
            exit 1
        }
    done
done

mkdir -p "$root/.tmp"
tmp=$(mktemp -d "$root/.tmp/quackgis-multi.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

concurrent_read() {
    edge=$1
    query "$edge" "WITH held AS MATERIALIZED (SELECT pg_sleep(3)) SELECT (SELECT worker_id FROM remote.worker_identity_export LIMIT 1) || '|' || count(*) || '|' || sum(id) FROM held CROSS JOIN public.features /* quackgis-multi-concurrent */"
}

active_readers() {
    edge=$1
    "$compose" exec -T "$edge" psql \
        -XAt \
        -v ON_ERROR_STOP=1 \
        -U postgres \
        -d quackgis \
        -c "SELECT count(*) FROM pg_stat_activity WHERE usename = 'quackgis_reader' AND state = 'active' AND query LIKE '%quackgis-multi-concurrent%'"
}

concurrent_read postgres_a >"$tmp/a1" &
pid_a1=$!
concurrent_read postgres_a >"$tmp/a2" &
pid_a2=$!
concurrent_read postgres_b >"$tmp/b1" &
pid_b1=$!
concurrent_read postgres_b >"$tmp/b2" &
pid_b2=$!

sleep 1
[ "$(active_readers postgres_a)" = "2" ] || {
    echo "edge A did not hold two simultaneous client sessions" >&2
    exit 1
}
[ "$(active_readers postgres_b)" = "2" ] || {
    echo "edge B did not hold two simultaneous client sessions" >&2
    exit 1
}

wait "$pid_a1"
wait "$pid_a2"
wait "$pid_b1"
wait "$pid_b2"

for result in "$tmp/a1" "$tmp/a2"; do
    [ "$(cat "$result")" = "worker-a|3|6" ] || {
        echo "concurrent edge A client returned an unexpected result" >&2
        exit 1
    }
done
for result in "$tmp/b1" "$tmp/b2"; do
    [ "$(cat "$result")" = "worker-b|3|6" ] || {
        echo "concurrent edge B client returned an unexpected result" >&2
        exit 1
    }
done

rm -rf "$tmp"
trap - EXIT INT TERM

echo "QuackGIS shared DuckLake passed: snapshot=$snapshot_id files=$data_file_count workers=2 clients=4"
