#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the minimal Kind topology without persisting plaintext secrets."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
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
        "@@BOOTSTRAP_SECRET_KEY@@",
        "@@WORKER_SECRET_KEY@@",
        "@@CREDENTIAL_SECRET_KEY@@",
        "@@CLIENT_TRANSPORT_SECRET_KEY@@",
        "@@BOOTSTRAP_PUBLIC_KEY@@",
        "@@WORKER_PUBLIC_KEY@@",
        "@@CREDENTIAL_PUBLIC_KEY@@",
        "@@PACKAGE_CONFIG_HASH@@",
    },
    "clients.yaml.in": {
        "@@CLIENT_IMAGE@@",
        "@@TLS_CA_CERTIFICATE@@",
        "@@CLIENT_TLS_CERTIFICATE@@",
        "@@CLIENT_TLS_PRIVATE_KEY@@",
    },
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
        "quackgis-bootstrap",
        "quackgis-worker-edge",
        "quackgis-client",
        "name: quackgis-edge-internal",
        "publishNotReadyAddresses: true",
        'args: ["--host", "127.0.0.1", "--port", "5434"]',
        "local_tls",
        "client_ca_path",
        "disable_relays",
        "path: /readyz",
        "path: /healthz",
    ]:
        if required not in core:
            raise ValueError(f"core template is missing {required!r}")
    for required in [
        "quackgis-psql",
        "quackgis-psycopg",
        "psycopg_copied_data_ok",
        "COPY {table} (id, name, geom_wkb) FROM STDIN",
        "quackgis-ogr",
        "quackgis-direct-denied",
        "quackgis-plaintext-denied",
        "quackgis-uncredentialed-denied",
        "PGSSLCERT",
        "PGSSLKEY",
    ]:
        if required not in clients:
            raise ValueError(f"client template is missing {required!r}")
    if core.count("publishNotReadyAddresses: true") != 1:
        raise ValueError("only the internal edge Service may publish unready addresses")
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


def public_key(value: str, name: str) -> str:
    if not value or len(value) > 128 or any(character.isspace() for character in value):
        raise ValueError(f"{name} is not a bounded iroh public key")
    return value


def render(args: argparse.Namespace) -> None:
    check_templates()
    runtime_image = pinned_image(args.runtime_image, "--runtime-image")
    client_image = pinned_image(args.client_image, "--client-image")
    tls_dir = args.tls_dir.resolve()
    edge_dir = args.edge_dir.resolve()
    substitutions = {
        "@@RUNTIME_IMAGE@@": runtime_image,
        "@@CLIENT_IMAGE@@": client_image,
        "@@TLS_CERTIFICATE@@": encoded(tls_dir / "tls.crt"),
        "@@TLS_PRIVATE_KEY@@": encoded(tls_dir / "tls.key"),
        "@@TLS_CA_CERTIFICATE@@": encoded(tls_dir / "ca.crt"),
        "@@CLIENT_TLS_CERTIFICATE@@": encoded(tls_dir / "client.crt"),
        "@@CLIENT_TLS_PRIVATE_KEY@@": encoded(tls_dir / "client.key"),
        "@@BOOTSTRAP_SECRET_KEY@@": encoded(edge_dir / "bootstrap.key"),
        "@@WORKER_SECRET_KEY@@": encoded(edge_dir / "worker.key"),
        "@@CREDENTIAL_SECRET_KEY@@": encoded(edge_dir / "credential.key"),
        "@@CLIENT_TRANSPORT_SECRET_KEY@@": encoded(edge_dir / "client-transport.key"),
        "@@BOOTSTRAP_PUBLIC_KEY@@": public_key(
            args.bootstrap_public_key, "--bootstrap-public-key"
        ),
        "@@WORKER_PUBLIC_KEY@@": public_key(args.worker_public_key, "--worker-public-key"),
        "@@CREDENTIAL_PUBLIC_KEY@@": public_key(
            args.credential_public_key, "--credential-public-key"
        ),
    }
    package_hash = hashlib.sha256()
    package_hash.update(json.dumps(substitutions, sort_keys=True).encode("utf-8"))
    for template_name in sorted(EXPECTED_PLACEHOLDERS):
        template = (TEMPLATES / template_name).read_text(encoding="utf-8")
        package_hash.update(template.replace("@@PACKAGE_CONFIG_HASH@@", "").encode("utf-8"))
    substitutions["@@PACKAGE_CONFIG_HASH@@"] = package_hash.hexdigest()
    args.out_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.out_dir.chmod(0o700)
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
        temporary.unlink(missing_ok=True)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            output.write(text)
        temporary.replace(destination)
        destination.chmod(0o600)


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser()
    value.add_argument("--check", action="store_true")
    value.add_argument("--runtime-image")
    value.add_argument("--client-image")
    value.add_argument("--tls-dir", type=Path)
    value.add_argument("--edge-dir", type=Path)
    value.add_argument("--bootstrap-public-key")
    value.add_argument("--worker-public-key")
    value.add_argument("--credential-public-key")
    value.add_argument("--out-dir", type=Path)
    return value


def main() -> None:
    args = parser().parse_args()
    if args.check:
        check_templates()
        print("kind_template_check_ok topology=duckdb-only clients=6")
        return
    missing = [
        option
        for option in [
            "runtime_image",
            "client_image",
            "tls_dir",
            "edge_dir",
            "bootstrap_public_key",
            "worker_public_key",
            "credential_public_key",
            "out_dir",
        ]
        if getattr(args, option) is None
    ]
    if missing:
        names = ", ".join("--" + name.replace("_", "-") for name in missing)
        raise SystemExit(f"render requires: {names}")
    render(args)
    print(f"kind_render_ok out={args.out_dir}")


if __name__ == "__main__":
    main()
