#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the minimal Kind topology without persisting plaintext secrets."""

from __future__ import annotations

import argparse
import base64
import hashlib
import os
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent
TEMPLATES = ROOT / "templates"
IMAGE = re.compile(r"^[^\s@]+@sha256:[0-9a-f]{64}$")
QGIS_IMAGE = "docker.io/qgis/qgis@sha256:aa55ce7f4b87d8fd28accc51658fe550667865c2ed088778c35915c2b4347587"
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
        "@@REST_CREDENTIAL_PUBLIC_KEY@@",
        "@@CORE_CONFIG_HASH@@",
    },
    "rest.yaml.in": {
        "@@RUNTIME_IMAGE@@",
        "@@JWT_SECRET@@",
        "@@REST_CREDENTIAL_SECRET_KEY@@",
        "@@BOOTSTRAP_PUBLIC_KEY@@",
        "@@REST_CONFIG_HASH@@",
    },
    "rest-seed.yaml.in": {
        "@@CLIENT_IMAGE@@",
        "@@TLS_CA_CERTIFICATE@@",
        "@@CLIENT_TLS_CERTIFICATE@@",
        "@@CLIENT_TLS_PRIVATE_KEY@@",
    },
    "clients.yaml.in": {
        "@@CLIENT_IMAGE@@",
        "@@TLS_CA_CERTIFICATE@@",
        "@@CLIENT_TLS_CERTIFICATE@@",
        "@@CLIENT_TLS_PRIVATE_KEY@@",
    },
    "qgis.yaml.in": {
        "@@QGIS_IMAGE@@",
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
    rest = (TEMPLATES / "rest.yaml.in").read_text(encoding="utf-8")
    seed = (TEMPLATES / "rest-seed.yaml.in").read_text(encoding="utf-8")
    qgis = (TEMPLATES / "qgis.yaml.in").read_text(encoding="utf-8")
    for required in [
        "kind: StatefulSet",
        "replicas: 1",
        "persistentVolumeReclaimPolicy: Retain",
        "quackgis-bootstrap",
        "quackgis-worker-edge",
        "quackgis-client",
        "name: quackgis-edge-internal",
        "name: quackgis-edge-access",
        "publishNotReadyAddresses: true",
        'args: ["--host", "127.0.0.1", "--port", "5434"]',
        "local_tls",
        "client_ca_path",
        "disable_relays",
        "path: /readyz",
        "path: /healthz",
        '"login_role": "authenticator"',
        "value: edge-preauthenticated",
        "name: quackgis-roles",
    ]:
        if required not in core:
            raise ValueError(f"core template is missing {required!r}")
    for required in [
        "quackgis-psql",
        "psql_describe_copied_data_ok",
        "quackgis-psycopg",
        "psycopg_copied_data_ok",
        "COPY {table} (id, name, geom_wkb) FROM STDIN",
        "quackgis-ogr",
        "ogr_copied_data_ok",
        "ogr_direct_discovery_ok",
        'ST_GeomFromWKB(geom_wkb) AS "ST_AsEWKB"',
        "quackgis-direct-denied",
        "quackgis-plaintext-denied",
        "quackgis-uncredentialed-denied",
        "PGSSLCERT",
        "PGSSLKEY",
    ]:
        if required not in clients:
            raise ValueError(f"client template is missing {required!r}")
    for required in [
        "kind: Deployment",
        "replicas: 2",
        "name: quackgis-rest-client",
        '"listen": "127.0.0.1:5432"',
        "value: edge-preauthenticated",
        "name: quackgis-rest",
        "path: /ready",
        "path: /live",
    ]:
        if required not in rest:
            raise ValueError(f"REST template is missing {required!r}")
    for required in ["name: quackgis-rest-seed", "kind_rest_points", "kind_rest_seed_ok"]:
        if required not in seed:
            raise ValueError(f"REST seed template is missing {required!r}")
    for required in [
        "name: quackgis-qgis",
        "3.44.11-Solothurn",
        "qgis_query_layer_ok",
        "QgsDataSourceUri.SslVerifyFull",
        "public.kind_psycopg_points",
    ]:
        if required not in qgis:
            raise ValueError(f"QGIS template is missing {required!r}")
    if core.count("publishNotReadyAddresses: true") != 1:
        raise ValueError("only the internal edge Service may publish unready addresses")
    forbidden = ["datafusion", "sedona", "linkerd", "minio", "postgresql"]
    qgis_topology = qgis.replace("postgresql", "")
    combined = f"{core}\n{clients}\n{rest}\n{seed}\n{qgis_topology}".lower()
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
        "@@QGIS_IMAGE@@": pinned_image(QGIS_IMAGE, "QGIS image"),
        "@@TLS_CERTIFICATE@@": encoded(tls_dir / "tls.crt"),
        "@@TLS_PRIVATE_KEY@@": encoded(tls_dir / "tls.key"),
        "@@TLS_CA_CERTIFICATE@@": encoded(tls_dir / "ca.crt"),
        "@@CLIENT_TLS_CERTIFICATE@@": encoded(tls_dir / "client.crt"),
        "@@CLIENT_TLS_PRIVATE_KEY@@": encoded(tls_dir / "client.key"),
        "@@BOOTSTRAP_SECRET_KEY@@": encoded(edge_dir / "bootstrap.key"),
        "@@WORKER_SECRET_KEY@@": encoded(edge_dir / "worker.key"),
        "@@CREDENTIAL_SECRET_KEY@@": encoded(edge_dir / "credential.key"),
        "@@CLIENT_TRANSPORT_SECRET_KEY@@": encoded(edge_dir / "client-transport.key"),
        "@@REST_CREDENTIAL_SECRET_KEY@@": encoded(edge_dir / "rest-credential.key"),
        "@@JWT_SECRET@@": encoded(args.jwt_secret_file.resolve()),
        "@@BOOTSTRAP_PUBLIC_KEY@@": public_key(
            args.bootstrap_public_key, "--bootstrap-public-key"
        ),
        "@@WORKER_PUBLIC_KEY@@": public_key(args.worker_public_key, "--worker-public-key"),
        "@@CREDENTIAL_PUBLIC_KEY@@": public_key(
            args.credential_public_key, "--credential-public-key"
        ),
        "@@REST_CREDENTIAL_PUBLIC_KEY@@": public_key(
            args.rest_credential_public_key, "--rest-credential-public-key"
        ),
    }
    args.out_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.out_dir.chmod(0o700)
    for source_name, output_name in [
        ("runtime.yaml.in", "core.yaml"),
        ("rest-seed.yaml.in", "rest-seed.yaml"),
        ("rest.yaml.in", "rest.yaml"),
        ("clients.yaml.in", "clients.yaml"),
        ("qgis.yaml.in", "qgis.yaml"),
    ]:
        text = (TEMPLATES / source_name).read_text(encoding="utf-8")
        for marker, value in substitutions.items():
            text = text.replace(marker, value)
        config_marker = {
            "runtime.yaml.in": "@@CORE_CONFIG_HASH@@",
            "rest.yaml.in": "@@REST_CONFIG_HASH@@",
        }.get(source_name)
        if config_marker is not None:
            digest = hashlib.sha256(text.replace(config_marker, "").encode("utf-8"))
            text = text.replace(config_marker, digest.hexdigest())
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
    value.add_argument("--rest-credential-public-key")
    value.add_argument("--jwt-secret-file", type=Path)
    value.add_argument("--out-dir", type=Path)
    return value


def main() -> None:
    args = parser().parse_args()
    if args.check:
        check_templates()
        print("kind_template_check_ok topology=duckdb-only clients=6 optional=qgis copied_data=psycopg,ogr")
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
