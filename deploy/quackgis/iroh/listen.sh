#!/bin/sh
set -eu

ticket=/tunnel/worker.ticket
ticket_tmp=$ticket.tmp
stderr_fifo=/tmp/dumbpipe.stderr

rm -f "$ticket" "$ticket_tmp" "$stderr_fifo"
mkfifo "$stderr_fifo"

cleanup() {
    trap - EXIT INT TERM
    [ -z "${listener_pid:-}" ] || kill "$listener_pid" 2>/dev/null || true
    [ -z "${reader_pid:-}" ] || kill "$reader_pid" 2>/dev/null || true
    rm -f "$stderr_fifo" "$ticket_tmp"
}
trap cleanup EXIT INT TERM

dumbpipe listen-tcp --host 127.0.0.1:9494 2>"$stderr_fifo" &
listener_pid=$!

(
    while IFS= read -r line; do
        printf '%s\n' "$line" >&2
        case "$line" in
            'dumbpipe connect-tcp '*)
                printf '%s\n' "${line#dumbpipe connect-tcp }" >"$ticket_tmp"
                mv "$ticket_tmp" "$ticket"
                ;;
        esac
    done <"$stderr_fifo"
) &
reader_pid=$!

wait "$listener_pid"
