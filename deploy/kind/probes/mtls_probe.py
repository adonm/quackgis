# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import sys
import time
import urllib.request

import lb_probe
import storage_probe
from probe_common import require


def linkerd_metrics() -> str:
    with urllib.request.urlopen("http://127.0.0.1:4191/metrics", timeout=10) as response:
        return response.read().decode("utf-8", errors="replace")


def main() -> int:
    storage_status = storage_probe.main()
    require(storage_status == 0, f"storage probe failed with status {storage_status}")
    lb_status = lb_probe.main()
    require(lb_status == 0, f"load-balance probe failed with status {lb_status}")
    time.sleep(1.0)
    metrics = linkerd_metrics()
    tls_true = 'tls="true"' in metrics or 'tls="tls"' in metrics
    require(tls_true, "Linkerd proxy metrics did not report TLS traffic")
    print("mtls_metrics_tls", True)
    print("mtls_ok", True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
