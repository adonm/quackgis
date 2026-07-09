# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import json
import sys


def is_ready(pod: dict) -> bool:
    return any(
        condition.get("type") == "Ready" and condition.get("status") == "True"
        for condition in pod.get("status", {}).get("conditions", [])
    )


def has_linkerd_proxy(pod: dict) -> bool:
    return any(
        container.get("name") == "linkerd-proxy"
        for container in pod.get("spec", {}).get("containers", [])
    )


def main() -> int:
    expected_apps = sys.argv[1:]
    if not expected_apps:
        raise RuntimeError("usage: check_linkerd_injected.py <app> [<app> ...]")
    payload = json.load(sys.stdin)
    pods = payload.get("items", [])
    for app in expected_apps:
        matching = [
            pod
            for pod in pods
            if pod.get("metadata", {}).get("labels", {}).get("app") == app
            and is_ready(pod)
            and has_linkerd_proxy(pod)
        ]
        if not matching:
            raise RuntimeError(f"no ready Linkerd-injected pod found for app={app}")
        names = ",".join(pod["metadata"]["name"] for pod in matching)
        print("linkerd_injected", f"app={app}", f"pods={names}")
    print("linkerd_injected_ok", True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
