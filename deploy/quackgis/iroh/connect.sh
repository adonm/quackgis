#!/bin/sh
set -eu

ticket_file=/tunnel/worker.ticket

while [ ! -s "$ticket_file" ]; do
    sleep 1
done

exec dumbpipe connect-tcp --addr 127.0.0.1:9494 "$(cat "$ticket_file")"
