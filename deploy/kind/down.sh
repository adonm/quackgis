#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
  engine=$(python3 "$root/scripts/project_doctor.py" --container-engine)
fi
: "${KIND_EXPERIMENTAL_PROVIDER:=$engine}"
export KIND_EXPERIMENTAL_PROVIDER

kind delete cluster --name quackgis
printf 'kind_down_ok provider=%s cluster=quackgis\n' "$KIND_EXPERIMENTAL_PROVIDER"
