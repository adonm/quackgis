#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Report project tooling and select one usable local container engine."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Tool:
    command: str
    purpose: str
    install: str
    group: str


TOOLS = (
    Tool("git", "source control", "host package", "core"),
    Tool("python3", "project scripts and probes", "host package", "core"),
    Tool("mise", "pinned tool bootstrap", "https://mise.jdx.dev", "core"),
    Tool("cargo", "Rust build and tests", "mise install", "core"),
    Tool("rustc", "Rust compiler", "mise install", "core"),
    Tool("just", "project recipes", "mise install", "core"),
    Tool("duckdb", "native engine probes", "mise install", "core"),
    Tool("openssl", "development TLS material", "host package", "kind"),
    Tool("kind", "local Kubernetes cluster", "mise install", "kind"),
    Tool("kubectl", "Kubernetes API/client jobs", "mise install", "kind"),
    Tool("podman", "rootless container/Kind provider", "host package", "container"),
    Tool("docker", "alternative container/Kind provider", "host package", "container"),
    Tool("curl", "optional REST manual client", "host package", "client"),
    Tool("psql", "optional host PostgreSQL client", "Kind client image or host package", "client"),
    Tool("ogrinfo", "optional host GDAL/OGR client", "Kind client image or host package", "client"),
    Tool("qgis_process", "optional headless QGIS qualification", "future QGIS client image or host package", "client"),
)


def usable_engine(command: str) -> bool:
    if shutil.which(command) is None:
        return False
    try:
        subprocess.run(
            [command, "info"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return True


def select_container_engine() -> str:
    override = os.environ.get("CONTAINER_ENGINE") or os.environ.get(
        "KIND_EXPERIMENTAL_PROVIDER"
    )
    if override:
        if override not in {"podman", "docker"}:
            raise RuntimeError(
                f"CONTAINER_ENGINE must be podman or docker, got {override!r}"
            )
        if not usable_engine(override):
            raise RuntimeError(f"requested container engine is not usable: {override}")
        return override
    for candidate in ("podman", "docker"):
        if usable_engine(candidate):
            return candidate
    raise RuntimeError(
        "no usable container engine; install/start Podman or Docker, or set CONTAINER_ENGINE"
    )


def version(command: str) -> str:
    probes = {
        "kubectl": [command, "version", "--client"],
        "openssl": [command, "version"],
        "python3": [command, "--version"],
        "psql": [command, "--version"],
        "ogrinfo": [command, "--version"],
        "qgis_process": [command, "--version"],
    }
    args = probes.get(command, [command, "--version"])
    try:
        result = subprocess.run(
            args,
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError):
        return "installed (version probe failed)"
    output = (result.stdout or result.stderr).strip().splitlines()
    return output[0][:100] if output else "installed"


def podman_host_summary() -> str | None:
    if not usable_engine("podman"):
        return None
    try:
        result = subprocess.run(
            ["podman", "info", "--format", "json"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        import json

        host = json.loads(result.stdout).get("host", {})
        return "rootless={} cgroup={} runtime={}".format(
            host.get("security", {}).get("rootless"),
            host.get("cgroupVersion", "unknown"),
            host.get("ociRuntime", {}).get("name", "unknown"),
        )
    except (OSError, ValueError, subprocess.SubprocessError):
        return "installed (host capability probe failed)"


def report() -> None:
    print("QuackGIS project tools")
    print("STATUS   TOOL          GROUP      PURPOSE / VERSION")
    for tool in TOOLS:
        path = shutil.which(tool.command)
        if path:
            print(
                f"ok       {tool.command:<13} {tool.group:<10} "
                f"{tool.purpose}; {version(tool.command)}"
            )
        else:
            print(
                f"missing  {tool.command:<13} {tool.group:<10} "
                f"{tool.purpose}; install: {tool.install}"
            )
    try:
        engine = select_container_engine()
        print(f"\ncontainer_engine={engine} (override with CONTAINER_ENGINE=podman|docker)")
    except RuntimeError as error:
        print(f"\ncontainer_engine=missing ({error})")
    podman = podman_host_summary()
    if podman:
        print(f"podman_host={podman}")
    print("host psql/OGR/QGIS are optional when their digest-pinned Kind client images are used")


def check_group(group: str) -> int:
    required_groups = {"core"} if group == "core" else {"core", "kind"}
    missing = [
        tool.command
        for tool in TOOLS
        if tool.group in required_groups and shutil.which(tool.command) is None
    ]
    if group == "kind":
        try:
            select_container_engine()
        except RuntimeError as error:
            print(f"doctor_error {error}", file=sys.stderr)
            return 1
    if missing:
        print(f"doctor_missing group={group} tools={','.join(missing)}", file=sys.stderr)
        return 1
    print(f"doctor_ok group={group}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--container-engine", action="store_true")
    parser.add_argument("--check", choices=("core", "kind"))
    args = parser.parse_args()
    if args.container_engine:
        try:
            print(select_container_engine())
        except RuntimeError as error:
            print(error, file=sys.stderr)
            return 1
        return 0
    if args.check:
        return check_group(args.check)
    report()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
