#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
secret="$work/rest/jwt-secret"
previous="$work/previous-rest-jwt"
rendered="$work/rendered"
kubeconfig=${KUBECONFIG:-$work/kubeconfig}
export KUBECONFIG="$kubeconfig"

if [ ! -f "$secret" ]; then
  printf 'current Kind REST JWT key is missing: %s\n' "$secret" >&2
  exit 2
fi
if [ -e "$previous" ]; then
  printf 'refusing JWT rotation while previous material exists: %s\n' "$previous" >&2
  exit 2
fi
old_token=$(JWT_SECRET_FILE="$secret" python3 - <<'PY'
import base64, hashlib, hmac, json, os, time
enc = lambda value: base64.urlsafe_b64encode(value).rstrip(b"=").decode()
header = enc(json.dumps({"alg":"HS256","typ":"JWT"}, separators=(",", ":")).encode())
claims = enc(json.dumps({
    "iss":"https://kind.quackgis.test", "aud":"quackgis-rest",
    "sub":"kind-old-key-gate", "role":"rest_reader", "exp":int(time.time()) + 300,
}, separators=(",", ":")).encode())
secret = open(os.environ["JWT_SECRET_FILE"], "rb").read().strip(b" \t\n\r\v\f")
signature = enc(hmac.new(secret, f"{header}.{claims}".encode(), hashlib.sha256).digest())
print(f"{header}.{claims}.{signature}")
PY
)
runtime_image=$(kubectl -n quackgis get statefulset quackgis -o jsonpath='{.spec.template.spec.containers[0].image}')
client_image=$(python3 -c 'import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"image: \"([^\"]+)\"", text)
if match is None:
    raise SystemExit("cannot find rendered client image")
print(match.group(1))' "$rendered/clients.yaml")
engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
  engine=$(python3 "$root/scripts/project_doctor.py" --container-engine)
fi
mv "$secret" "$previous"
openssl rand -hex 48 >"$secret"
chmod 600 "$secret"
if ! CONTAINER_ENGINE="$engine" \
  QUACKGIS_RUNTIME_IMAGE="$runtime_image" \
  QUACKGIS_CLIENT_IMAGE="$client_image" \
  "$root/deploy/kind/up.sh"; then
  printf 'REST JWT rollout failed; previous material retained at %s\n' "$previous" >&2
  exit 1
fi
QUACKGIS_OLD_JWT="$old_token" "$root/deploy/kind/rest-gates.sh"
printf 'kind_rest_jwt_rotation_staged old_tokens=denied replicas=2 previous_material=%s\n' "$previous"
