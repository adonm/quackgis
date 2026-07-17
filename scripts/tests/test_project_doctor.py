#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import importlib.util
import json
import os
import sys
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/project_doctor.py"
SPEC = importlib.util.spec_from_file_location("project_doctor", MODULE_PATH)
assert SPEC and SPEC.loader
project_doctor = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = project_doctor
SPEC.loader.exec_module(project_doctor)


def main() -> None:
    podman_server = {
        "Server": {"Components": [{"Name": "Podman Engine", "Version": "5.8.4"}]}
    }
    docker_server = {
        "Server": {"Components": [{"Name": "Engine", "Version": "29.6.1"}]}
    }
    with (
        mock.patch.object(project_doctor.shutil, "which", return_value="/usr/bin/docker"),
        mock.patch.object(
            project_doctor.subprocess,
            "run",
            return_value=mock.Mock(stdout=json.dumps(podman_server)),
        ),
    ):
        assert project_doctor.docker_server_backend() == "podman"
    with (
        mock.patch.object(project_doctor.shutil, "which", return_value="/usr/bin/docker"),
        mock.patch.object(
            project_doctor.subprocess,
            "run",
            return_value=mock.Mock(stdout=json.dumps(docker_server)),
        ),
    ):
        assert project_doctor.docker_server_backend() == "docker"
    with (
        mock.patch.dict(os.environ, {"CONTAINER_ENGINE": "docker"}, clear=True),
        mock.patch.object(project_doctor, "usable_engine", return_value=True),
        mock.patch.object(project_doctor, "docker_server_backend", return_value="podman"),
    ):
        try:
            project_doctor.select_container_engine()
        except RuntimeError as error:
            assert "connected to Podman" in str(error)
        else:
            raise AssertionError("Docker-over-Podman override was accepted")
    print("project_doctor_test_ok")


if __name__ == "__main__":
    main()
