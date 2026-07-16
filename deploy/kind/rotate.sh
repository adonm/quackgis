#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work="$root/.tmp/kind"
tls="$work/tls"
edge="$work/edge"
previous="$work/previous-tls"
previous_edge="$work/previous-edge"
rendered="$work/rendered"
kubeconfig=${KUBECONFIG:-$work/kubeconfig}
export KUBECONFIG="$kubeconfig"

engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
  engine=$(python3 "$root/scripts/project_doctor.py" --container-engine)
fi
: "${KIND_EXPERIMENTAL_PROVIDER:=$engine}"
export KIND_EXPERIMENTAL_PROVIDER

if [ -e "$previous" ] || [ -e "$previous_edge" ]; then
  printf 'refusing rotation while retained previous material exists under %s\n' "$work" >&2
  exit 2
fi
for file in ca.crt client.crt client.key; do
  if [ ! -f "$tls/$file" ]; then
    printf 'current Kind TLS material is incomplete: %s\n' "$tls/$file" >&2
    exit 2
  fi
done

runtime_image=$(kubectl -n quackgis get statefulset quackgis -o jsonpath='{.spec.template.spec.containers[0].image}')
client_image=$(python3 -c 'import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"image: \"([^\"]+)\"", text)
if match is None:
    raise SystemExit("cannot find rendered client image")
print(match.group(1))' "$rendered/clients.yaml")

mv "$tls" "$previous"
mv "$edge" "$previous_edge"
if ! CONTAINER_ENGINE="$engine" \
  QUACKGIS_RUNTIME_IMAGE="$runtime_image" \
  QUACKGIS_CLIENT_IMAGE="$client_image" \
  "$root/deploy/kind/up.sh"; then
  printf 'rotation rollout failed; previous TLS and edge material retained under %s\n' "$work" >&2
  exit 1
fi

cleanup() {
  kubectl -n quackgis delete job quackgis-old-client-denied --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n quackgis delete secret quackgis-kind-old-client-tls --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n quackgis delete job quackgis-old-rest-credential-denied --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n quackgis delete secret quackgis-kind-old-rest-edge --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n quackgis delete configmap quackgis-kind-old-rest-edge --ignore-not-found >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup
kubectl -n quackgis create secret generic quackgis-kind-old-client-tls \
  --from-file=tls.crt="$previous/client.crt" \
  --from-file=tls.key="$previous/client.key" \
  --from-file=ca.crt="$tls/ca.crt" >/dev/null
cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: quackgis-old-client-denied
  namespace: quackgis
spec:
  backoffLimit: 0
  template:
    spec:
      restartPolicy: Never
      securityContext:
        fsGroup: 65532
        fsGroupChangePolicy: OnRootMismatch
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: old-client-denied
          image: "$client_image"
          imagePullPolicy: IfNotPresent
          command: ["/bin/sh", "-ceu"]
          args:
            - >-
              if PGCONNECT_TIMEOUT=3 PGSSLMODE=verify-full
              PGSSLROOTCERT=/etc/quackgis/tls/ca.crt
              PGSSLCERT=/etc/quackgis/tls/tls.crt
              PGSSLKEY=/etc/quackgis/tls/tls.key
              psql -h quackgis.quackgis.svc.cluster.local -p 5432
              -U postgres -d quackgis -c 'SELECT 1'; then
                echo 'old client certificate unexpectedly succeeded' >&2; exit 1;
              fi
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            runAsNonRoot: true
            runAsUser: 65532
            runAsGroup: 65532
            seccompProfile:
              type: RuntimeDefault
          volumeMounts:
            - name: tls
              mountPath: /etc/quackgis/tls
              readOnly: true
      volumes:
        - name: tls
          secret:
            secretName: quackgis-kind-old-client-tls
            defaultMode: 288
EOF
if ! kubectl -n quackgis wait --for=condition=complete job/quackgis-old-client-denied --timeout=2m; then
  kubectl -n quackgis logs job/quackgis-old-client-denied --all-containers=true || true
  printf 'old client denial gate failed; previous TLS material retained at %s\n' "$previous" >&2
  exit 1
fi
kubectl -n quackgis logs job/quackgis-old-client-denied --all-containers=true

keygen=${QUACKGIS_KEYGEN:-$root/target/release/quackgis-keygen}
if [ ! -x "$keygen" ]; then
  printf 'quackgis-keygen is required to verify the old REST credential\n' >&2
  exit 2
fi
bootstrap_public_key=$("$keygen" --public-from "$edge/bootstrap.key")
kubectl -n quackgis create secret generic quackgis-kind-old-rest-edge \
  --from-file=credential.key="$previous_edge/rest-credential.key" >/dev/null
old_rest_config=$(cat <<EOF
{
  "credential_secret_key_path": "/var/run/quackgis-old-rest-edge/credential.key",
  "transport_secret_key_path": "/var/run/quackgis-old-rest-edge/client-transport.key",
  "bootstrap": {
    "endpoint_id": "$bootstrap_public_key",
    "direct_hosts": ["quackgis-edge-internal.quackgis.svc.cluster.local:4243"]
  },
  "listen": "127.0.0.1:5432",
  "disable_relays": true,
  "bind": "0.0.0.0:0",
  "max_connections": 4
}
EOF
)
kubectl -n quackgis create configmap quackgis-kind-old-rest-edge \
  --from-literal=client.json="$old_rest_config" >/dev/null
cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: quackgis-old-rest-credential-denied
  namespace: quackgis
spec:
  backoffLimit: 0
  template:
    spec:
      restartPolicy: Never
      securityContext:
        fsGroup: 999
        fsGroupChangePolicy: OnRootMismatch
        seccompProfile:
          type: RuntimeDefault
      initContainers:
        - name: prepare-edge
          image: "$runtime_image"
          imagePullPolicy: IfNotPresent
          command: ["/bin/sh", "-ceu"]
          args:
            - >-
              cp /source/credential.key /keys/credential.key;
              /usr/local/bin/quackgis-keygen --out /keys/client-transport.key >/dev/null;
              chmod 600 /keys/*.key
          securityContext: &old_rest_security
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            runAsNonRoot: true
            runAsUser: 999
            runAsGroup: 999
            seccompProfile:
              type: RuntimeDefault
          volumeMounts:
            - {name: old-secret, mountPath: /source, readOnly: true}
            - {name: keys, mountPath: /keys}
        - name: old-rest-edge
          restartPolicy: Always
          image: "$runtime_image"
          imagePullPolicy: IfNotPresent
          command: ["/usr/local/bin/quackgis-client"]
          args: ["--config", "/config/client.json"]
          securityContext: *old_rest_security
          volumeMounts:
            - {name: config, mountPath: /config, readOnly: true}
            - {name: keys, mountPath: /var/run/quackgis-old-rest-edge, readOnly: true}
      containers:
        - name: denial
          image: "$client_image"
          imagePullPolicy: IfNotPresent
          command: ["/bin/sh", "-ceu"]
          args:
            - >-
              python3 -c 'import socket,time
              for attempt in range(50):
                  try:
                      socket.create_connection(("127.0.0.1",5432),1).close(); break
                  except OSError:
                      time.sleep(.1)
              else: raise SystemExit("old credential bridge did not listen")';
              if PGCONNECT_TIMEOUT=5 psql -h 127.0.0.1 -p 5432
              -U authenticator -d quackgis -c 'SELECT 1'; then
                echo 'old REST credential unexpectedly obtained a lease' >&2; exit 1;
              fi;
              echo old_rest_credential_denied
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            runAsNonRoot: true
            runAsUser: 65532
            runAsGroup: 65532
            seccompProfile:
              type: RuntimeDefault
      volumes:
        - name: old-secret
          secret:
            secretName: quackgis-kind-old-rest-edge
            defaultMode: 288
        - name: config
          configMap:
            name: quackgis-kind-old-rest-edge
            defaultMode: 292
        - name: keys
          emptyDir: {}
EOF
if ! kubectl -n quackgis wait --for=condition=complete job/quackgis-old-rest-credential-denied --timeout=2m; then
  kubectl -n quackgis logs job/quackgis-old-rest-credential-denied --all-containers=true || true
  printf 'old REST credential denial gate failed; previous edge material retained at %s\n' "$previous_edge" >&2
  exit 1
fi
kubectl -n quackgis logs job/quackgis-old-rest-credential-denied --all-containers=true
printf 'kind_secret_rotation_staged old_client=denied old_rest_credential=denied previous_material=%s\n' "$work"
