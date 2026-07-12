#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the minimal Kind topology without persisting plaintext secrets."""

from __future__ import annotations

import argparse
import base64
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent
TEMPLATES = ROOT / "templates"
IMAGE = re.compile(r"^[^\s@]+@sha256:[0-9a-f]{64}$")
EXPECTED_PLACEHOLDERS = {
    "runtime.yaml.in": {
        "@@RUNTIME_IMAGE@@",
        "@@TLS_CERTIFICATE@@",
        "@@TLS_PRIVATE_KEY@@",
        "@@TLS_CA_CERTIFICATE@@",
        "@@AUTH_PASSWORD@@",
    },
    "clients.yaml.in": {"@@CLIENT_IMAGE@@"},
}


def placeholders(text: str) -> set[str]:
    return set(re.findall(r"@@[A-Z_]+@@", text))


def check_templates() -> None:
    for name, expected in EXPECTED_PLACEHOLDERS.items():
        text = (TEMPLATES / name).read_text(encoding="utf-8")
        actual = placeholders(text)
        if actual != expected:
            raise ValueError(f"{name} placeholders {sorted(actual)} != {sorted(expected)}")
    core = (TEMPLATES / "runtime.yaml.in").read_text(encoding="utf-8")
    clients = (TEMPLATES / "clients.yaml.in").read_text(encoding="utf-8")
    for required in [
        "kind: StatefulSet",
        "replicas: 1",
        "persistentVolumeReclaimPolicy: Retain",
        "QUACKGIS_TLS_MODE",
        "value: required",
        "path: /readyz",
        "path: /healthz",
    ]:
        if required not in core:
            raise ValueError(f"core template is missing {required!r}")
    for required in ["quackgis-psql", "quackgis-psycopg", "quackgis-ogr"]:
        if required not in clients:
            raise ValueError(f"client template is missing {required!r}")
    forbidden = ["datafusion", "sedona", "linkerd", "minio", "postgresql"]
    combined = f"{core}\n{clients}".lower()
    present = [value for value in forbidden if value in combined]
    if present:
        raise ValueError(f"retired/deferred topology names present: {present}")


def pinned_image(value: str, name: str) -> str:
    if not IMAGE.fullmatch(value):
        raise ValueError(f"{name} must be an immutable image@sha256 digest reference")
    return value


def encoded(path: Path) -> str:
    data = path.read_bytes()
    if not data:
        raise ValueError(f"secret input is empty: {path}")
    return base64.b64encode(data).decode("ascii")


def render(args: argparse.Namespace) -> None:
    check_templates()
    runtime_image = pinned_image(args.runtime_image, "--runtime-image")
    client_image = pinned_image(args.client_image, "--client-image")
    tls_dir = args.tls_dir.resolve()
    substitutions = {
        "@@RUNTIME_IMAGE@@": runtime_image,
        "@@CLIENT_IMAGE@@": client_image,
        "@@TLS_CERTIFICATE@@": encoded(tls_dir / "tls.crt"),
        "@@TLS_PRIVATE_KEY@@": encoded(tls_dir / "tls.key"),
        "@@TLS_CA_CERTIFICATE@@": encoded(tls_dir / "ca.crt"),
        "@@AUTH_PASSWORD@@": encoded(args.password_file.resolve()),
    }
    args.out_dir.mkdir(parents=True, exist_ok=True)
    for source_name, output_name in [
        ("runtime.yaml.in", "core.yaml"),
        ("clients.yaml.in", "clients.yaml"),
    ]:
        text = (TEMPLATES / source_name).read_text(encoding="utf-8")
        for marker, value in substitutions.items():
            text = text.replace(marker, value)
        unresolved = placeholders(text)
        if unresolved:
            raise ValueError(f"unresolved placeholders in {source_name}: {sorted(unresolved)}")
        destination = args.out_dir / output_name
        temporary = destination.with_suffix(".yaml.tmp")
        temporary.write_text(text, encoding="utf-8")
        temporary.replace(destination)


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser()
    value.add_argument("--check", action="store_true")
    value.add_argument("--runtime-image")
    value.add_argument("--client-image")
    value.add_argument("--tls-dir", type=Path)
    value.add_argument("--password-file", type=Path)
    value.add_argument("--out-dir", type=Path)
    return value


def main() -> None:
    args = parser().parse_args()
    if args.check:
        check_templates()
        print("kind_template_check_ok topology=duckdb-only clients=3")
        return
    missing = [
        option
        for option in ["runtime_image", "client_image", "tls_dir", "password_file", "out_dir"]
        if getattr(args, option) is None
    ]
    if missing:
        names = ", ".join("--" + name.replace("_", "-") for name in missing)
        raise SystemExit(f"render requires: {names}")
    render(args)
    print(f"kind_render_ok out={args.out_dir}")


if __name__ == "__main__":
    main()
