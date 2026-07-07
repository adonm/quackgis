#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Wait for a TCP listener, optionally failing early if a watched PID exits."""

from __future__ import annotations

import os
import socket
import sys
import time
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 5:
        print("usage: wait_for_tcp.py HOST PORT PID LOG", file=sys.stderr)
        return 2
    host = sys.argv[1]
    port = int(sys.argv[2])
    pid = int(sys.argv[3])
    log = Path(sys.argv[4])
    deadline = time.time() + 60
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return 0
        except OSError:
            try:
                os.kill(pid, 0)
            except OSError:
                print("watched process exited before TCP listener was ready", file=sys.stderr)
                if log.exists():
                    print(log.read_text(encoding="utf-8", errors="replace"), file=sys.stderr)
                return 1
            time.sleep(0.5)
    print(f"timed out waiting for {host}:{port}", file=sys.stderr)
    if log.exists():
        print(log.read_text(encoding="utf-8", errors="replace"), file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
