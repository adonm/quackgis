#!/bin/sh
set -eu

source=/usr/local/share/quackgis/.duckdb/extensions
target=/var/lib/postgresql/.duckdb/extensions

mkdir -p "$target"
cp -R "$source/." "$target/"
