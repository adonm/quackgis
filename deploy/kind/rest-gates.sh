#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
kubeconfig=${KUBECONFIG:-$work/kubeconfig}
export KUBECONFIG="$kubeconfig"
secret="$work/rest/jwt-secret"
old_token=${QUACKGIS_OLD_JWT:-}
if [ ! -f "$secret" ]; then
  printf 'Kind REST JWT secret is missing: %s\n' "$secret" >&2
  exit 2
fi

reader_token=$(JWT_SECRET_FILE="$secret" JWT_ROLE=rest_reader python3 - <<'PY'
import base64, hashlib, hmac, json, os, time
enc = lambda value: base64.urlsafe_b64encode(value).rstrip(b"=").decode()
header = enc(json.dumps({"alg":"HS256","typ":"JWT"}, separators=(",", ":")).encode())
claims = enc(json.dumps({
    "iss":"https://kind.quackgis.test", "aud":"quackgis-rest",
    "sub":"kind-gate", "role":os.environ["JWT_ROLE"], "exp":int(time.time()) + 300,
}, separators=(",", ":")).encode())
secret = open(os.environ["JWT_SECRET_FILE"], "rb").read().strip(b" \t\n\r\v\f")
signature = enc(hmac.new(secret, f"{header}.{claims}".encode(), hashlib.sha256).digest())
print(f"{header}.{claims}.{signature}")
PY
)
denied_token=$(JWT_SECRET_FILE="$secret" JWT_ROLE=rest_denied python3 - <<'PY'
import base64, hashlib, hmac, json, os, time
enc = lambda value: base64.urlsafe_b64encode(value).rstrip(b"=").decode()
header = enc(json.dumps({"alg":"HS256","typ":"JWT"}, separators=(",", ":")).encode())
claims = enc(json.dumps({
    "iss":"https://kind.quackgis.test", "aud":"quackgis-rest",
    "sub":"kind-gate", "role":os.environ["JWT_ROLE"], "exp":int(time.time()) + 300,
}, separators=(",", ":")).encode())
secret = open(os.environ["JWT_SECRET_FILE"], "rb").read().strip(b" \t\n\r\v\f")
signature = enc(hmac.new(secret, f"{header}.{claims}".encode(), hashlib.sha256).digest())
print(f"{header}.{claims}.{signature}")
PY
)

forward_pid=
cleanup() {
  if [ -n "$forward_pid" ]; then
    kill "$forward_pid" >/dev/null 2>&1 || true
    wait "$forward_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

start_forward() {
  resource=$1
  local_port=$2
  log="$work/port-forward-${local_port}.log"
  rm -f "$log"
  kubectl -n quackgis port-forward "$resource" "$local_port:3000" >"$log" 2>&1 &
  forward_pid=$!
  attempt=0
  while ! grep -q 'Forwarding from' "$log" 2>/dev/null; do
    if ! kill -0 "$forward_pid" 2>/dev/null; then
      cat "$log" >&2
      exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
      cat "$log" >&2
      printf 'REST port-forward did not become ready\n' >&2
      exit 1
    fi
    sleep 0.05
  done
}

stop_forward() {
  cleanup
  forward_pid=
}

check_endpoint() {
  base=$1
  BASE_URL="$base" READER_TOKEN="$reader_token" DENIED_TOKEN="$denied_token" OLD_TOKEN="$old_token" python3 - <<'PY'
import json, os, urllib.error, urllib.request
base = os.environ["BASE_URL"]

def request(path, token=None, method="GET"):
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    try:
        with urllib.request.urlopen(
            urllib.request.Request(base + path, headers=headers, method=method), timeout=5
        ) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()

reader = os.environ["READER_TOKEN"]
denied = os.environ["DENIED_TOKEN"]
status, body = request("/kind_rest_points?select=id,name&order=id.asc", reader)
assert status == 200 and json.loads(body) == [
    {"id": 1, "name": "one"}, {"id": 2, "name": "two"}
], (status, body)
status, body = request("/", reader)
assert status == 200 and "/kind_rest_points" in json.loads(body)["paths"], (status, body)
status, body = request("/", denied)
assert status == 200 and "/kind_rest_points" not in json.loads(body)["paths"], (status, body)
assert request("/kind_rest_points", denied)[0] == 404
assert request("/kind_rest_points")[0] == 401
assert request("/kind_rest_points", reader, "POST")[0] == 405
assert request("/ready")[0] == 200
old = os.environ.get("OLD_TOKEN")
if old:
    assert request("/kind_rest_points", old)[0] == 401
PY
}

wait_ready() {
  base=$1
  BASE_URL="$base" python3 - <<'PY'
import os, time, urllib.error, urllib.request
url = os.environ["BASE_URL"] + "/ready"
for _ in range(30):
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            if response.status == 200:
                break
    except (OSError, urllib.error.HTTPError):
        pass
    time.sleep(0.5)
else:
    raise SystemExit("REST endpoint did not recover readiness")
PY
}

pods=$(kubectl -n quackgis get pods -l app.kubernetes.io/name=quackgis-rest \
  --field-selector=status.phase=Running -o name | sort)
pod_count=$(printf '%s\n' "$pods" | sed '/^$/d' | wc -l)
if [ "$pod_count" -ne 2 ]; then
  printf 'expected two running REST Pods, got %s\n' "$pod_count" >&2
  exit 1
fi
port=13001
for pod in $pods; do
  start_forward "$pod" "$port"
  wait_ready "http://127.0.0.1:$port"
  check_endpoint "http://127.0.0.1:$port"
  stop_forward
  port=$((port + 1))
done

ready_endpoints=$(kubectl -n quackgis get endpointslice \
  -l kubernetes.io/service-name=quackgis-rest -o json | python3 -c '
import json, sys
value=json.load(sys.stdin)
print(sum(1 for item in value["items"] for endpoint in item.get("endpoints", []) if endpoint.get("conditions", {}).get("ready") is not False))')
if [ "$ready_endpoints" -ne 2 ]; then
  printf 'expected two ready REST endpoints, got %s\n' "$ready_endpoints" >&2
  exit 1
fi

removed=$(printf '%s\n' "$pods" | head -n 1)
kubectl -n quackgis delete "$removed" --wait=false >/dev/null
attempt=0
while :; do
  remaining=$(kubectl -n quackgis get endpointslice \
    -l kubernetes.io/service-name=quackgis-rest -o json | python3 -c '
import json, sys
value=json.load(sys.stdin)
print(sum(1 for item in value["items"] for endpoint in item.get("endpoints", []) if endpoint.get("conditions", {}).get("ready") is not False))')
  if [ "$remaining" -ge 1 ]; then break; fi
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then
    printf 'REST Service lost every ready endpoint after one Pod deletion\n' >&2
    exit 1
  fi
  sleep 0.25
done
start_forward service/quackgis-rest 13010
wait_ready http://127.0.0.1:13010
check_endpoint http://127.0.0.1:13010
stop_forward
kubectl -n quackgis rollout status deployment/quackgis-rest --timeout=3m
printf 'kind_rest_gates_ok replicas=2 endpoints=2 failover=passed roles=reader,denied\n'
