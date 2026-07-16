#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import importlib.util
import stat
import tempfile
from argparse import Namespace
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "deploy/kind/render.py"
SPEC = importlib.util.spec_from_file_location("kind_render", MODULE_PATH)
assert SPEC and SPEC.loader
kind_render = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(kind_render)


def main() -> None:
    kind_render.check_templates()
    cluster = (ROOT / "deploy/kind/cluster.yaml").read_text(encoding="utf-8")
    up = (ROOT / "deploy/kind/up.sh").read_text(encoding="utf-8")
    rotate = (ROOT / "deploy/kind/rotate.sh").read_text(encoding="utf-8")
    clients_image = (ROOT / "deploy/Containerfile.kind-clients").read_text(
        encoding="utf-8"
    )
    assert "kindest/node:v1.36.1@sha256:" in cluster
    assert "KIND_EXPERIMENTAL_PROVIDER" in up
    assert "kind load image-archive" in up
    assert "--format docker-archive" in up
    assert 'if [ "$engine" = podman ]' in up
    assert "QUACKGIS_RUNTIME_LOAD_IMAGE" in up
    assert "QUACKGIS_CLIENT_LOAD_IMAGE" in up
    assert "kind_cluster_stale" in up
    assert "get --raw=/readyz" in up
    assert "kind_pod_replace" in up
    assert "kind_statefulset_replace" in up
    assert "quackgis-old-client-denied" in rotate
    assert "previous-edge" in rotate
    assert "FROM registry.fedoraproject.org/fedora-minimal@sha256:" in clients_image
    for client in [
        "postgresql-18.3-2.fc43.x86_64",
        "python3-psycopg3-3.2.13-1.fc43.noarch",
        "python3-psycopg3_c-3.2.13-1.fc43.x86_64",
        "gdal-3.11.5-1.fc43.x86_64",
    ]:
        assert client in clients_image
    assert "USER 65532:65532" in clients_image
    digest = "example.invalid/image@sha256:" + "a" * 64
    assert kind_render.pinned_image(digest, "image") == digest
    try:
        kind_render.pinned_image("example.invalid/image:latest", "image")
    except ValueError:
        pass
    else:
        raise AssertionError("mutable image tag was accepted")

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        tls = root / "tls"
        tls.mkdir()
        for name in ["tls.crt", "tls.key", "ca.crt", "client.crt", "client.key"]:
            (tls / name).write_text(name, encoding="utf-8")
        edge = root / "edge"
        edge.mkdir()
        for name in ["bootstrap", "worker", "credential", "client-transport"]:
            (edge / f"{name}.key").write_text(f"{name}-secret", encoding="utf-8")
        output = root / "rendered"
        kind_render.render(
            Namespace(
                runtime_image=digest,
                client_image=digest.replace("image", "clients"),
                tls_dir=tls,
                edge_dir=edge,
                bootstrap_public_key="bootstrap-public",
                worker_public_key="worker-public",
                credential_public_key="credential-public",
                out_dir=output,
            )
        )
        rendered = (output / "core.yaml").read_text(encoding="utf-8")
        clients = (output / "clients.yaml").read_text(encoding="utf-8")
        assert "@@" not in rendered + clients
        assert digest in rendered
        assert "Ym9vdHN0cmFwLXNlY3JldA==" in rendered
        assert '"listen": "0.0.0.0:5432"' in rendered
        assert "quackgis-edge-internal.quackgis.svc.cluster.local" in rendered
        assert rendered.count("publishNotReadyAddresses: true") == 1
        assert '"local_tls"' in rendered
        assert 'args: ["--host", "127.0.0.1", "--port", "5434"]' in rendered
        assert "sslmode=verify-full" in clients
        assert "PGSSLCERT" in clients
        assert "psycopg_copied_data_ok" in clients
        assert "kind_psycopg_points" in clients
        assert "quackgis-direct-denied" in clients
        assert "quackgis-plaintext-denied" in clients
        assert "quackgis-uncredentialed-denied" in clients
        assert stat.S_IMODE(output.stat().st_mode) == 0o700
        assert stat.S_IMODE((output / "core.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "clients.yaml").stat().st_mode) == 0o600
    print("kind_render_test_ok")


if __name__ == "__main__":
    main()
