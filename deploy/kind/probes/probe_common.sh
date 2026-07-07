#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Shared shell helpers for Kind client compatibility probes.

set -euo pipefail

probe_table_from_uid() {
  local prefix="$1"
  local suffix="${POD_UID:-$(date +%s)}"
  suffix="${suffix//-/_}"
  printf '%s_%s' "${prefix}" "${suffix}"
}

probe_curl_auth() {
  curl -fsS -u "${GEOSERVER_ADMIN_USER}:${GEOSERVER_ADMIN_PASSWORD}" "$@"
}

probe_wait_geoserver() {
  local base="$1"
  local pid="$2"
  for _ in $(seq 1 180); do
    if probe_curl_auth "${base}/rest/about/version.json" >/tmp/version.json 2>/tmp/geoserver-ready.err; then
      return 0
    fi
    if ! kill -0 "${pid}" 2>/dev/null; then
      printf 'GeoServer exited before readiness\n'
      test -f /tmp/geoserver-ready.err && cat /tmp/geoserver-ready.err || true
      return 1
    fi
    sleep 2
  done
  printf 'GeoServer did not become ready\n'
  test -f /tmp/geoserver-ready.err && cat /tmp/geoserver-ready.err || true
  return 1
}

probe_xml_success() {
  local path="$1"
  grep -Eq 'SUCCESS|totalInserted="?1|totalUpdated="?1|totalDeleted="?1' "${path}"
}
