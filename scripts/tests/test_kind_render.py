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
    assert 'deploy/kind/load-image.sh"' in up
    assert "QUACKGIS_RUNTIME_LOAD_IMAGE" in up
    assert "QUACKGIS_CLIENT_LOAD_IMAGE" in up
    assert "kind_cluster_stale" in up
    assert "reason=%s action=recreate" in up
    assert "stale_reason=kubeconfig-export" in up
    assert "get --raw=/readyz" in up
    assert "kind_pod_replace" in up
    assert "kind_statefulset_replace" in up
    load_image = (ROOT / "deploy/kind/load-image.sh").read_text(encoding="utf-8")
    assert "kind load image-archive" in load_image
    assert "--format docker-archive" in load_image
    assert 'if [ "$engine" = podman ]' in load_image
    assert "invalid image archive name" in load_image
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
        for name in [
            "tls.crt",
            "tls.key",
            "ca.crt",
            "client.crt",
            "client.key",
            "migration-ca.crt",
            "migration-client.crt",
            "migration-client.key",
        ]:
            (tls / name).write_text(name, encoding="utf-8")
        edge = root / "edge"
        edge.mkdir()
        for name in [
            "bootstrap",
            "worker",
            "credential",
            "client-transport",
            "rest-credential",
            "migration-credential",
            "migration-transport",
        ]:
            (edge / f"{name}.key").write_text(f"{name}-secret", encoding="utf-8")
        jwt_secret = root / "jwt-secret"
        jwt_secret.write_text("bounded-jwt-secret", encoding="utf-8")
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
                rest_credential_public_key="rest-credential-public",
                migration_credential_public_key="migration-credential-public",
                jwt_secret_file=jwt_secret,
                out_dir=output,
            )
        )
        rendered = (output / "core.yaml").read_text(encoding="utf-8")
        clients = (output / "clients.yaml").read_text(encoding="utf-8")
        rest = (output / "rest.yaml").read_text(encoding="utf-8")
        seed = (output / "rest-seed.yaml").read_text(encoding="utf-8")
        qgis = (output / "qgis.yaml").read_text(encoding="utf-8")
        migration = (output / "migration.yaml").read_text(encoding="utf-8")
        migration_clients = (output / "migration-clients.yaml").read_text(encoding="utf-8")
        migration_qgis = (output / "migration-qgis.yaml").read_text(encoding="utf-8")
        assert "@@" not in (
            rendered
            + clients
            + rest
            + seed
            + qgis
            + migration
            + migration_clients
            + migration_qgis
        )
        assert digest in rendered
        assert "Ym9vdHN0cmFwLXNlY3JldA==" in rendered
        assert '"listen": "0.0.0.0:5432"' in rendered
        assert "quackgis-edge-internal.quackgis.svc.cluster.local" in rendered
        assert "name: quackgis-edge-access" in rendered
        assert rendered.count("publishNotReadyAddresses: true") == 1
        assert '"local_tls"' in rendered
        assert 'args: ["--host", "127.0.0.1", "--port", "5434"]' in rendered
        assert '"login_role": "postgres"' in rendered
        assert '"login_role": "authenticator"' in rendered
        assert '"login_role": "migration_operator"' in rendered
        assert "migration-credential-public" in rendered
        assert "name: quackgis-migration" in rendered
        assert '"listen": "0.0.0.0:5433"' in rendered
        assert "migration-ca.crt" not in rendered
        assert "value: edge-preauthenticated" in rendered
        assert "kind_psycopg_points" in rendered
        assert "kind_rest_points" in rendered
        assert "rest-credential-public" in rendered
        assert "kind: Deployment" in rest
        assert "replicas: 2" in rest
        assert '"listen": "127.0.0.1:5432"' in rest
        assert "quackgis-edge-access.quackgis.svc.cluster.local" in rest
        assert "postgres://authenticator@127.0.0.1:5432/quackgis" in rest
        assert "QUACKGIS_REST_DATABASE_PASSWORD_FILE" not in rest
        assert "quackgis-data" not in rest
        assert "kind_rest_seed_ok" in seed
        assert "sslmode=verify-full" in clients
        assert "PGSSLCERT" in clients
        assert "psql_describe_copied_data_ok" in clients
        assert "psycopg_copied_data_ok" in clients
        assert "ogr_copied_data_ok" in clients
        assert "ogr_copy_write_ok" in clients
        assert "PG_USE_COPY YES" in clients
        assert "qgis_query_layer_ok" in qgis
        assert "QgsMapRendererParallelJob" in qgis
        assert "setFilterExpression" in qgis
        assert "setFilterRect" in qgis
        assert "render_pixels" in qgis
        assert "3.44.11-Solothurn" in qgis
        assert "public.kind_psycopg_points" in qgis
        assert "QgsDataSourceUri.SslVerifyFull" in qgis
        assert "@sha256:" in qgis
        assert "ogr_direct_discovery_ok" in clients
        assert 'ST_GeomFromWKB(geom_wkb) AS "ST_AsEWKB"' in clients
        assert "kind_psycopg_points" in clients
        assert "kind_ogr_points" in clients
        assert "quackgis-direct-denied" in clients
        assert "quackgis-plaintext-denied" in clients
        assert "quackgis-uncredentialed-denied" in clients
        assert "quackgis-postgis-migration" in migration
        assert "/usr/local/bin/quackgis-migrate" in migration
        assert "migration_operator" in migration
        assert "cleanup-configured-targets" in migration
        assert "kind_postgis_migration_ok" in migration
        assert "kind_migration_public_certificate_denied" in migration
        assert "restartPolicy: Always" in migration
        assert "postgis/postgis@sha256:" in migration
        assert "bWlncmF0aW9uLWNsaWVudC5jcnQ=" in migration
        assert "reset-configured-targets" in migration
        assert "--staging-id g0stage" in migration
        assert "--runtime-manifest /opt/quackgis/artifact-manifest.json" in migration
        assert "migration_psql_ok" in migration_clients
        assert "migration_psycopg_ok" in migration_clients
        assert "migration_ogr_ok" in migration_clients
        assert "migration_operator" in migration_clients
        assert "migration_qgis_ok" in migration_qgis
        assert "QgsMapRendererParallelJob" in migration_qgis
        first_core = rendered
        first_rest = rest
        jwt_secret.write_text("different-bounded-jwt-secret-value", encoding="utf-8")
        kind_render.render(
            Namespace(
                runtime_image=digest,
                client_image=digest.replace("image", "clients"),
                tls_dir=tls,
                edge_dir=edge,
                bootstrap_public_key="bootstrap-public",
                worker_public_key="worker-public",
                credential_public_key="credential-public",
                rest_credential_public_key="rest-credential-public",
                migration_credential_public_key="migration-credential-public",
                jwt_secret_file=jwt_secret,
                out_dir=output,
            )
        )
        assert (output / "core.yaml").read_text(encoding="utf-8") == first_core
        assert (output / "rest.yaml").read_text(encoding="utf-8") != first_rest
        assert stat.S_IMODE(output.stat().st_mode) == 0o700
        assert stat.S_IMODE((output / "core.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "clients.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "rest.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "rest-seed.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "migration.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "migration-clients.yaml").stat().st_mode) == 0o600
        assert stat.S_IMODE((output / "migration-qgis.yaml").stat().st_mode) == 0o600
    print("kind_render_test_ok")


if __name__ == "__main__":
    main()
