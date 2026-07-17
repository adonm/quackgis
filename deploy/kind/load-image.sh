#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
  engine=$(python3 "$root/scripts/project_doctor.py" --container-engine)
fi
case "$engine" in
  podman|docker) ;;
  *) printf 'unsupported Kind container engine: %s\n' "$engine" >&2; exit 2 ;;
esac
if [ "$#" -ne 2 ]; then
  printf 'usage: %s SOURCE_IMAGE ARCHIVE_NAME\n' "$0" >&2
  exit 2
fi
source_image=$1
archive_name=$2
case "$archive_name" in
  ''|*[!A-Za-z0-9._-]*) printf 'invalid image archive name: %s\n' "$archive_name" >&2; exit 2 ;;
esac

archive="$work/$archive_name.tar"
mkdir -p "$work"
rm -f "$archive"
trap 'rm -f "$archive"' EXIT HUP INT TERM
if [ "$engine" = podman ]; then
  "$engine" image save --format docker-archive --output "$archive" "$source_image"
else
  "$engine" image save --output "$archive" "$source_image"
fi
KIND_EXPERIMENTAL_PROVIDER="$engine" kind load image-archive "$archive" --name quackgis
printf 'kind_image_load_ok provider=%s image=%s\n' "$engine" "$source_image"
