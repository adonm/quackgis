#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run the G0 snapshot migrator against pinned PostGIS and actual QuackGIS."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import threading
import time

PINNED_POSTGIS_IMAGE = (
    "docker.io/postgis/postgis@"
    "sha256:3813864c8321c36dbbf6e9cfd27926006923d9afe41ca5e5294092833b7f2ca1"
)


def run(
    command: list[str],
    *,
    env: dict[str, str] | None = None,
    input_text: str | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        env=env,
        input=input_text,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_for_socket(port: int, process: subprocess.Popen[str], log_path: Path) -> None:
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(
                f"QuackGIS exited before readiness:\n{log_path.read_text(encoding='utf-8')}"
            )
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError("QuackGIS did not open its pgwire listener within 60 seconds")


def source_psql(
    engine: str,
    container: str,
    sql: str,
    *,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return run(
        [engine, "exec", "-i", container, "psql", "-XAt", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "fixture"],
        input_text=sql,
        check=check,
    )


def target_psql(engine: str, image: str, port: int, sql: str) -> str:
    result = run(
        [
            engine,
            "run",
            "--rm",
            "--network",
            "host",
            image,
            "psql",
            "-XAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-h",
            "127.0.0.1",
            "-p",
            str(port),
            "-U",
            "postgres",
            "-d",
            "quackgis",
            "-c",
            sql,
        ]
    )
    return result.stdout.strip()


def wait_for_postgis(engine: str, container: str) -> tuple[int, str]:
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        result = source_psql(
            engine,
            container,
            "SELECT current_setting('server_version_num'), public.postgis_lib_version();",
            check=False,
        )
        if result.returncode == 0 and "|" in result.stdout:
            version, postgis = result.stdout.strip().split("|", 1)
            return int(version), postgis
        time.sleep(0.25)
    raise RuntimeError("pinned PostGIS source did not finish initialization within 90 seconds")


def write_config(
    path: Path,
    postgres_version: int,
    postgis_version: str,
    tables: list[dict[str, object]],
    role_mappings: dict[str, str] | None = None,
    grant_mappings: list[dict[str, object]] | None = None,
) -> None:
    path.write_text(
        json.dumps(
            {
                "format_version": 1,
                "source": {
                    "postgres_version_num": postgres_version,
                    "postgis_version": postgis_version,
                },
                "source_schemas": ["public", "survey"],
                "tables": tables,
                "role_mappings": role_mappings or {},
                "grant_mappings": grant_mappings or [],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def migration_environment(
    source_port: int,
    target_port: int,
    password_file: Path,
    application_name: str,
) -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "QUACKGIS_MIGRATE_SOURCE_URL": (
                f"postgresql://postgres@127.0.0.1:{source_port}/fixture"
                f"?application_name={application_name}"
            ),
            "QUACKGIS_MIGRATE_SOURCE_PASSWORD_FILE": str(password_file),
            "QUACKGIS_MIGRATE_TARGET_URL": (
                f"postgresql://postgres@127.0.0.1:{target_port}/quackgis"
            ),
        }
    )
    return env


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--container-engine", default=os.environ.get("CONTAINER_ENGINE", "podman"))
    parser.add_argument("--postgis-image", default=PINNED_POSTGIS_IMAGE)
    parser.add_argument("--server-bin", type=Path, default=Path("target/debug/quackgis-server"))
    parser.add_argument("--migrate-bin", type=Path, default=Path("target/debug/quackgis-migrate"))
    parser.add_argument("--driver", type=Path, required=True)
    parser.add_argument("--duckdb-home", type=Path, default=Path(".tmp/duckdb/home"))
    args = parser.parse_args()

    root = args.workspace.resolve()
    engine = args.container_engine
    server_bin = (root / args.server_bin).resolve()
    migrate_bin = (root / args.migrate_bin).resolve()
    driver = (root / args.driver).resolve()
    duckdb_home = (root / args.duckdb_home).resolve()
    for path, label in [(server_bin, "server"), (migrate_bin, "migrator"), (driver, "ADBC driver")]:
        if not path.is_file():
            raise RuntimeError(f"{label} is missing: {path}")
    if not duckdb_home.is_dir():
        raise RuntimeError(f"DuckDB extension home is missing: {duckdb_home}")

    work = root / ".tmp" / "postgis-migration-smoke"
    shutil.rmtree(work, ignore_errors=True)
    (work / "target-data").mkdir(parents=True)
    password = secrets.token_urlsafe(24)
    password_file = work / "source-password"
    password_file.write_text(password, encoding="utf-8")
    password_file.chmod(0o600)

    source_port = free_port()
    target_port = free_port()
    container = f"quackgis-g0-{os.getpid()}"
    server: subprocess.Popen[str] | None = None
    server_log = work / "server.log"
    log_handle = None
    try:
        run([engine, "pull", args.postgis_image])
        run(
            [
                engine,
                "run",
                "-d",
                "--rm",
                "--name",
                container,
                "-e",
                f"POSTGRES_PASSWORD={password}",
                "-e",
                "POSTGRES_DB=fixture",
                "-p",
                f"127.0.0.1:{source_port}:5432",
                args.postgis_image,
            ]
        )
        postgres_version, postgis_version = wait_for_postgis(engine, container)
        if postgres_version != 180_004 or not postgis_version.startswith("3.6."):
            raise RuntimeError(
                f"pinned source resolved to unexpected versions {postgres_version}/{postgis_version}"
            )

        fixture_sql = """
CREATE SCHEMA survey;
CREATE TABLE public.places (
  id BIGINT NOT NULL DEFAULT 7,
  label TEXT,
  amount NUMERIC(10,2) NOT NULL DEFAULT 12.34,
  observed_on DATE,
  observed_at TIMESTAMP(6),
  payload BYTEA,
  location geometry(Point, 0)
);
COMMENT ON TABLE public.places IS 'migration fixture';
COMMENT ON COLUMN public.places.label IS 'display label';
INSERT INTO public.places VALUES
  (1, E'one\\\\tline', 1.25, DATE '2026-07-18',
   TIMESTAMP '2026-07-18 01:02:03.123456', decode('00ff','hex'),
   ST_GeomFromText('POINT(1 2)', 0)),
  (2, NULL, 2.50, NULL, NULL, NULL, NULL);
CREATE TABLE survey.readings(id INTEGER, enabled BOOLEAN, ratio REAL);
INSERT INTO survey.readings
SELECT value, value % 2 = 0, (value % 100)::REAL / 4
FROM generate_series(1, 100002) AS values(value);
CREATE VIEW public.place_view AS SELECT id, label FROM public.places;
CREATE SEQUENCE survey.unsupported_sequence;
CREATE ROLE source_reader;
GRANT SELECT(location) ON public.places TO source_reader;
CREATE TABLE public.keyed(id INTEGER PRIMARY KEY);
CREATE TABLE public.bad_dates(id INTEGER, observed_on DATE);
INSERT INTO public.bad_dates VALUES (1, 'infinity');
"""
        source_psql(engine, container, fixture_sql)

        log_handle = server_log.open("w", encoding="utf-8")
        server_env = os.environ.copy()
        server_env.update(
            {
                "HOME": str(duckdb_home),
                "QUACKGIS_DUCKDB_ADBC_DRIVER": str(driver),
            }
        )
        server = subprocess.Popen(
            [
                str(server_bin),
                "--host",
                "127.0.0.1",
                "--port",
                str(target_port),
                "--catalog-path",
                str(work / "target.ducklake"),
                "--data-path",
                str(work / "target-data"),
                "--auth-mode",
                "trust",
                "--log",
                "warn",
            ],
            cwd=root,
            env=server_env,
            text=True,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
        )
        wait_for_socket(target_port, server, server_log)

        tables = [
            {
                "source_schema": "public",
                "source_table": "places",
                "target_schema": "main",
                "target_table": "migrated_places",
                "column_mappings": {"location": "geom_wkb"},
            },
            {
                "source_schema": "survey",
                "source_table": "readings",
                "target_schema": "main",
                "target_table": "migrated_readings",
            },
        ]
        config = work / "migration.json"
        report_path = work / "migration-report.json"
        write_config(
            config,
            postgres_version,
            postgis_version,
            tables,
            {"source_reader": "reader"},
            [
                {
                    "source_role": "source_reader",
                    "source_schema": "public",
                    "source_table": "places",
                    "source_column": "location",
                    "privilege": "SELECT",
                    "target_role": "reader",
                }
            ],
        )

        inserted = threading.Event()
        watcher_error: list[str] = []

        def concurrent_writer() -> None:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                result = source_psql(
                    engine,
                    container,
                    "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity "
                    "WHERE application_name = 'quackgis-g0-snapshot' "
                    "AND backend_xmin IS NOT NULL);",
                    check=False,
                )
                if result.returncode == 0 and result.stdout.strip() == "t":
                    write = source_psql(
                        engine,
                        container,
                        "INSERT INTO survey.readings VALUES (200000, true, 9.5);",
                        check=False,
                    )
                    if write.returncode != 0:
                        watcher_error.append(write.stderr)
                    else:
                        inserted.set()
                    return
                time.sleep(0.01)
            watcher_error.append("source snapshot was not observed")

        watcher = threading.Thread(target=concurrent_writer, daemon=True)
        watcher.start()
        result = run(
            [
                str(migrate_bin),
                "run",
                "--config",
                str(config),
                "--out",
                str(report_path),
                "--allow-plaintext-loopback",
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(
                source_port, target_port, password_file, "quackgis-g0-snapshot"
            ),
            check=False,
        )
        watcher.join(timeout=30)
        if result.returncode != 0:
            raise RuntimeError(f"migration failed:\n{result.stderr}")
        if watcher_error or not inserted.is_set():
            raise RuntimeError(f"concurrent source writer failed: {watcher_error}")
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if report["state"] != "verified":
            raise RuntimeError(f"migration did not verify: {report['errors']}")
        evidence = {table["source_identity"]: table for table in report["tables"]}
        mapped_roles = {role["identity"]: role for role in report["preflight"]["roles"]}
        mapped_grants = report["preflight"]["grants"]
        if (
            mapped_roles["source_reader"]["target_identity"] != "reader"
            or mapped_roles["source_reader"]["disposition"]["action"] != "map"
            or not any(
                grant["target_identity"] == "reader:main.migrated_places.geom_wkb"
                and grant["disposition"]["action"] == "map"
                for grant in mapped_grants
            )
        ):
            raise RuntimeError("explicit source role/grant mapping was not reported exactly")
        if evidence["public.places"]["rows"] != 2:
            raise RuntimeError("Point/NULL fixture row count changed")
        if evidence["survey.readings"]["rows"] != 100_002:
            raise RuntimeError("repeatable-read snapshot admitted the concurrent source write")
        source_count = source_psql(
            engine, container, "SELECT count(*) FROM survey.readings;"
        ).stdout.strip()
        if source_count != "100003":
            raise RuntimeError(f"concurrent source write was not committed: {source_count}")
        target_count = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT, sum(id)::BIGINT FROM public.migrated_readings;",
        )
        if target_count != "100002|5000250003":
            raise RuntimeError(f"target exact snapshot aggregate differs: {target_count}")
        spatial = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT, count(geom_wkb)::BIGINT, "
            "sum(octet_length(geom_wkb))::BIGINT FROM public.migrated_places;",
        )
        if spatial != "2|1|21":
            raise RuntimeError(f"target WKB/NULL aggregate differs: {spatial}")
        serialized_report = report_path.read_text(encoding="utf-8")
        for forbidden in [str(work), password, str(password_file), "postgresql://"]:
            if forbidden in serialized_report:
                raise RuntimeError("migration report contains an operational path or credential")

        failure_config = work / "failure.json"
        failure_report = work / "failure-report.json"
        write_config(
            failure_config,
            postgres_version,
            postgis_version,
            [
                {
                    "source_schema": "public",
                    "source_table": "places",
                    "target_schema": "main",
                    "target_table": "rollback_places",
                    "column_mappings": {"location": "geom_wkb"},
                },
                {
                    "source_schema": "public",
                    "source_table": "bad_dates",
                    "target_schema": "main",
                    "target_table": "rollback_bad_dates",
                },
            ],
        )
        failed = run(
            [
                str(migrate_bin),
                "run",
                "--config",
                str(failure_config),
                "--out",
                str(failure_report),
                "--allow-plaintext-loopback",
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-failure"),
            check=False,
        )
        failure = json.loads(failure_report.read_text(encoding="utf-8"))
        if failed.returncode == 0 or failure["state"] != "failed_rolled_back":
            raise RuntimeError("malformed source value did not fail and roll back migration")
        residue = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT FROM information_schema.tables "
            "WHERE table_schema = 'public' "
            "AND table_name IN ('rollback_places','rollback_bad_dates');",
        )
        if residue != "0":
            raise RuntimeError(f"failed migration left target table residue: {residue}")

        reject_config = work / "reject.json"
        reject_report = work / "reject-report.json"
        write_config(
            reject_config,
            postgres_version,
            postgis_version,
            [
                {
                    "source_schema": "public",
                    "source_table": "keyed",
                    "target_schema": "main",
                    "target_table": "must_not_connect",
                }
            ],
        )
        reject_env = migration_environment(source_port, 1, password_file, "g0-reject")
        rejected = run(
            [
                str(migrate_bin),
                "run",
                "--config",
                str(reject_config),
                "--out",
                str(reject_report),
                "--allow-plaintext-loopback",
                "--allow-plaintext-target-loopback",
            ],
            env=reject_env,
            check=False,
        )
        rejection = json.loads(reject_report.read_text(encoding="utf-8"))
        if rejected.returncode == 0 or rejection["state"] != "rejected":
            raise RuntimeError("unsupported primary key did not reject before target access")
        if not any(
            "PrimaryKey" in blocker
            for table in rejection["preflight"]["tables"]
            for blocker in table["blockers"]
        ):
            raise RuntimeError("primary-key rejection is absent from the report")

        cleanup_report = work / "cleanup-report.json"
        cleaned = run(
            [
                str(migrate_bin),
                "reset-configured-targets",
                "--config",
                str(config),
                "--out",
                str(cleanup_report),
                "--confirm-drop-configured-targets",
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-cleanup"),
            check=False,
        )
        cleanup = json.loads(cleanup_report.read_text(encoding="utf-8"))
        if cleaned.returncode != 0 or cleanup["dropped_configured_targets"] != [
            "main.migrated_places",
            "main.migrated_readings",
        ]:
            raise RuntimeError(f"configured-target cleanup failed: {cleaned.stderr}")
        cleanup_residue = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT FROM information_schema.tables "
            "WHERE table_schema = 'public' "
            "AND table_name IN ('migrated_places','migrated_readings');",
        )
        if cleanup_residue != "0":
            raise RuntimeError(
                f"configured-target cleanup left table residue: {cleanup_residue}"
            )

        def staged_run(report_file: Path, application_name: str) -> dict[str, object]:
            result = run(
                [
                    str(migrate_bin),
                    "run",
                    "--config",
                    str(config),
                    "--out",
                    str(report_file),
                    "--staging-id",
                    "g0stage",
                    "--allow-plaintext-loopback",
                    "--allow-plaintext-target-loopback",
                ],
                env=migration_environment(
                    source_port, target_port, password_file, application_name
                ),
                check=False,
            )
            if result.returncode != 0:
                raise RuntimeError(f"staged migration failed: {result.stderr}")
            return json.loads(report_file.read_text(encoding="utf-8"))

        staged_report_1_path = work / "staged-report-1.json"
        staged_report_1 = staged_run(staged_report_1_path, "g0-staged-1")
        staged_digest_1 = sha256(staged_report_1_path)
        wrong_digest = "0" * 64
        wrong_verify = run(
            [
                str(migrate_bin),
                "verify",
                "--report",
                str(staged_report_1_path),
                "--report-sha256",
                wrong_digest,
                "--out",
                str(work / "must-not-verify.json"),
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, 1, password_file, "g0-wrong-report"),
            check=False,
        )
        if wrong_verify.returncode == 0 or (work / "must-not-verify.json").exists():
            raise RuntimeError("wrong report digest did not reject before target access")

        staged_verify_1_path = work / "staged-verify-1.json"
        verified_1 = run(
            [
                str(migrate_bin),
                "verify",
                "--report",
                str(staged_report_1_path),
                "--report-sha256",
                staged_digest_1,
                "--out",
                str(staged_verify_1_path),
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-verify-1"),
            check=False,
        )
        staged_verify_1 = json.loads(staged_verify_1_path.read_text(encoding="utf-8"))
        if verified_1.returncode != 0 or staged_verify_1["state"] != "verified":
            raise RuntimeError(f"standalone staging verification failed: {verified_1.stderr}")

        staged_cleanup_path = work / "staged-cleanup.json"
        staged_cleanup = run(
            [
                str(migrate_bin),
                "cleanup",
                "--report",
                str(staged_report_1_path),
                "--report-sha256",
                staged_digest_1,
                "--out",
                str(staged_cleanup_path),
                "--confirm-cleanup-staging",
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-stage-cleanup"),
            check=False,
        )
        cleanup_stage = json.loads(staged_cleanup_path.read_text(encoding="utf-8"))
        if staged_cleanup.returncode != 0 or cleanup_stage["dropped_staging_targets"] != [
            "main.g0stage__migrated_places",
            "main.g0stage__migrated_readings",
        ]:
            raise RuntimeError(f"report-bound staging cleanup failed: {staged_cleanup.stderr}")
        lifecycle_residue = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT FROM information_schema.tables "
            "WHERE table_schema = 'public' AND table_name IN "
            "('migrated_places','migrated_readings',"
            "'g0stage__migrated_places','g0stage__migrated_readings');",
        )
        if lifecycle_residue != "0":
            raise RuntimeError("cleanup exposed a release or retained staging residue")

        staged_report_2_path = work / "staged-report-2.json"
        staged_report_2 = staged_run(staged_report_2_path, "g0-staged-2")
        checksums_1 = {
            table["source_identity"]: table["table_checksum"]
            for table in staged_report_1["tables"]
        }
        checksums_2 = {
            table["source_identity"]: table["table_checksum"]
            for table in staged_report_2["tables"]
        }
        if checksums_1 != checksums_2:
            raise RuntimeError("fresh staging retry produced different canonical checksums")
        staged_digest_2 = sha256(staged_report_2_path)
        staged_verify_2_path = work / "staged-verify-2.json"
        verified_2 = run(
            [
                str(migrate_bin),
                "verify",
                "--report",
                str(staged_report_2_path),
                "--report-sha256",
                staged_digest_2,
                "--out",
                str(staged_verify_2_path),
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-verify-2"),
            check=False,
        )
        if verified_2.returncode != 0:
            raise RuntimeError(f"retry staging verification failed: {verified_2.stderr}")
        verification_digest_2 = sha256(staged_verify_2_path)
        promotion_path = work / "promotion-report.json"
        promoted = run(
            [
                str(migrate_bin),
                "promote",
                "--report",
                str(staged_report_2_path),
                "--report-sha256",
                staged_digest_2,
                "--verification-report",
                str(staged_verify_2_path),
                "--verification-report-sha256",
                verification_digest_2,
                "--out",
                str(promotion_path),
                "--confirm-promote",
                "--allow-plaintext-target-loopback",
            ],
            env=migration_environment(source_port, target_port, password_file, "g0-promote"),
            check=False,
        )
        promotion = json.loads(promotion_path.read_text(encoding="utf-8"))
        if promoted.returncode != 0 or promotion["state"] != "promoted":
            raise RuntimeError(f"explicit promotion failed: {promoted.stderr}")
        promoted_state = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT, sum(id)::BIGINT FROM public.migrated_readings;",
        )
        promoted_staging_residue = target_psql(
            engine,
            args.postgis_image,
            target_port,
            "SELECT count(*)::BIGINT FROM information_schema.tables "
            "WHERE table_schema = 'public' AND table_name IN "
            "('g0stage__migrated_places','g0stage__migrated_readings');",
        )
        if promoted_state != "100003|5000450003" or promoted_staging_residue != "0":
            raise RuntimeError(
                "promoted release or staging cleanup differs: "
                f"{promoted_state}/{promoted_staging_residue}"
            )

        summary = {
            "postgres_version_num": postgres_version,
            "postgis_version": postgis_version,
            "state": report["state"],
            "snapshot_rows": evidence["survey.readings"]["rows"],
            "source_rows_after_concurrent_write": int(source_count),
            "wire_bytes": sum(table["wire_bytes"] for table in report["tables"]),
            "mapped_roles": 1,
            "mapped_grants": 1,
            "failure_state": failure["state"],
            "rejection_state": rejection["state"],
            "cleanup_targets": len(cleanup["dropped_configured_targets"]),
            "staging_retry": "identical",
            "promotion_state": promotion["state"],
        }
        print(json.dumps(summary, sort_keys=True))
        return 0
    finally:
        if server is not None:
            server.terminate()
            try:
                server.wait(timeout=15)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)
        if log_handle is not None:
            log_handle.close()
        run([engine, "rm", "-f", container], check=False)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"postgis migration smoke failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            if error.stdout:
                print(error.stdout, file=sys.stderr)
            if error.stderr:
                print(error.stderr, file=sys.stderr)
        raise SystemExit(1)
