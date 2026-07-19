#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Configure or build the prepared native bundle from one DuckDB checkout."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import native_bundle
import prepare_native_bundle


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PREPARED = ROOT / ".tmp/native-bundle"
PLAN_NAME = "central-build-plan.json"
BUILD_ENVIRONMENT_KEYS = {
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "LANG",
    "LC_ALL",
    "NO_PROXY",
    "PATH",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
}
VCPKG_GENERATED_PATHS = (
    "buildtrees",
    "downloads",
    "packages",
    "vcpkg",
    "vcpkg.disable-metrics",
)
TEST_RESULT = re.compile(
    r"All tests passed \((?P<assertions>[0-9]+) assertions? in "
    r"(?P<cases>[0-9]+) test cases?\)"
)
UPSTREAM_TEST_FILTERS = {
    "ducklake-functions": ("ducklake", "test/sql/functions/*"),
    "spatial-complete": ("spatial", "test/sql/*"),
}


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def run(
    command: list[str], *, cwd: Path | None = None, environment: dict[str, str] | None = None
) -> None:
    subprocess.run(command, cwd=cwd, env=environment, check=True)


def run_output(
    command: list[str], *, cwd: Path | None = None, environment: dict[str, str] | None = None
) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout + result.stderr


def parse_test_result(text: str, name: str) -> dict[str, int]:
    match = TEST_RESULT.search(text)
    if match is None:
        raise ValueError(f"cannot parse upstream test result for {name}")
    result = {
        "assertions": int(match.group("assertions")),
        "test_cases": int(match.group("cases")),
    }
    if result["assertions"] <= 0 or result["test_cases"] <= 0:
        raise ValueError(f"upstream test group {name} ran no assertions")
    return result


def candidate_test_filters(
    bundle: dict[str, Any], sources: Path
) -> dict[str, Path]:
    declared = bundle["test_groups"]["upstream"]
    unknown = sorted(set(declared) - set(UPSTREAM_TEST_FILTERS))
    if unknown:
        raise ValueError(f"native bundle declares unsupported upstream tests: {unknown}")
    filters = {
        name: sources / component / relative
        for name in declared
        for component, relative in [UPSTREAM_TEST_FILTERS[name]]
    }
    for component in native_bundle.COMPONENTS:
        series = native_bundle.validate_series(bundle, component, ROOT)
        for patch_index, patch in enumerate(series["patches"]):
            for test_index, relative in enumerate(patch["tests"]):
                if relative.endswith(".test"):
                    filters[f"patch-{component}-{patch_index}-{test_index}"] = (
                        sources / component / relative
                    )
    return filters


def candidate_test_requirement(name: str) -> str:
    if name == "ducklake-functions" or name.startswith("patch-ducklake-"):
        return "ducklake"
    if name == "spatial-complete" or name.startswith("patch-spatial-"):
        return "spatial"
    if name.startswith("patch-duckdb-"):
        raise ValueError("DuckDB core patch tests must not require an extension")
    raise ValueError(f"upstream test group has no extension requirement: {name}")


def tool_version(command: list[str], label: str) -> str:
    try:
        return output(command).splitlines()[0]
    except (OSError, subprocess.CalledProcessError, IndexError) as error:
        raise ValueError(f"required {label} tool is unavailable") from error


def resolve_tool_paths() -> dict[str, str]:
    tools = {
        "gcc": shutil.which("gcc", path=os.defpath),
        "g++": shutil.which("g++", path=os.defpath),
        "cmake": shutil.which("cmake"),
        "ninja": shutil.which("ninja"),
        "make": shutil.which("make", path=os.defpath),
    }
    missing = [name for name, path in tools.items() if not path]
    if missing:
        raise ValueError(f"required native build tools are unavailable: {missing}")
    return {name: str(Path(path).absolute()) for name, path in tools.items() if path}


def verify_toolchain(
    bundle: dict[str, Any], paths: dict[str, str]
) -> dict[str, Any]:
    compiler = bundle["toolchain"]["compiler"]
    if compiler["family"] != "gcc":
        raise ValueError("only the manifest-pinned GCC toolchain is currently supported")
    actual_cc = output([paths["gcc"], "-dumpfullversion", "-dumpversion"])
    actual_compiler = output([paths["g++"], "-dumpfullversion", "-dumpversion"])
    if actual_cc != compiler["version"] or actual_compiler != compiler["version"]:
        raise ValueError(
            f"compiler drift: expected GCC {compiler['version']}, got C={actual_cc} C++={actual_compiler}"
        )
    cmake = tool_version([paths["cmake"], "--version"], "CMake").removeprefix("cmake version ")
    ninja = tool_version([paths["ninja"], "--version"], "Ninja")
    make = tool_version([paths["make"], "--version"], "Make").removeprefix("GNU Make ")
    if cmake != bundle["toolchain"]["cmake_version"]:
        raise ValueError(f"CMake drift: expected {bundle['toolchain']['cmake_version']}, got {cmake}")
    if ninja != bundle["toolchain"]["ninja_version"]:
        raise ValueError(f"Ninja drift: expected {bundle['toolchain']['ninja_version']}, got {ninja}")
    if make != bundle["toolchain"]["make_version"]:
        raise ValueError(f"Make drift: expected {bundle['toolchain']['make_version']}, got {make}")
    actual_sha256 = {
        name: native_bundle.file_sha256(Path(path)) for name, path in paths.items()
    }
    if actual_sha256 != bundle["toolchain"]["executable_sha256"]:
        raise ValueError(
            "native build executable digest drift: "
            f"expected {bundle['toolchain']['executable_sha256']}, got {actual_sha256}"
        )
    return {
        "compiler": f"gcc {actual_compiler}",
        "cmake": cmake,
        "ninja": ninja,
        "make": make,
        "executable_sha256": actual_sha256,
        "acquisition": bundle["toolchain"]["acquisition"],
    }


def normalized_vcpkg_manifest(path: Path, spatial_source: Path) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    configuration = manifest.get("vcpkg-configuration")
    if not isinstance(configuration, dict):
        raise ValueError("merged vcpkg manifest is missing vcpkg-configuration")
    overlays = configuration.get("overlay-ports")
    if not isinstance(overlays, list):
        raise ValueError("merged vcpkg manifest is missing overlay ports")
    expected_overlay = (spatial_source / "vcpkg_ports").resolve()
    normalized = []
    for overlay in overlays:
        if not isinstance(overlay, str) or Path(overlay).resolve() != expected_overlay:
            raise ValueError(f"merged vcpkg manifest contains an unexpected overlay: {overlay!r}")
        normalized.append("${SPATIAL_SOURCE}/vcpkg_ports")
    configuration["overlay-ports"] = normalized
    return manifest


def make_environment(
    bundle: dict[str, Any], sources: Path, tool_paths: dict[str, str]
) -> dict[str, str]:
    version = bundle["duckdb"]["version"]
    commit = bundle["duckdb"]["source"]["commit"][:10]
    environment = {
        key: value for key, value in os.environ.items() if key in BUILD_ENVIRONMENT_KEYS
    }
    state = sources.parent / "build-state"
    tool_path = os.pathsep.join(
        dict.fromkeys(
            [
                str(Path(tool_paths[name]).parent)
                for name in ("cmake", "ninja", "make", "gcc", "g++")
            ]
            + ["/usr/bin", "/bin"]
        )
    )
    environment.update({
        "CC": tool_paths["gcc"],
        "CXX": tool_paths["g++"],
        "PATH": tool_path,
        "HOME": str((state / "home").absolute()),
        "TMPDIR": str((state / "tmp").absolute()),
        "XDG_CACHE_HOME": str((state / "xdg-cache").absolute()),
        "VCPKG_BINARY_SOURCES": "clear",
        "VCPKG_DEFAULT_BINARY_CACHE": str((state / "vcpkg-binary-cache").absolute()),
        "VCPKG_DOWNLOADS": str((state / "vcpkg-downloads").absolute()),
        "GEN": "ninja",
        "EXTENSION_CONFIGS": str((ROOT / bundle["build"]["extension_config"]).resolve()),
        "EXTRA_CMAKE_VARIABLES": " ".join(
            [
                f"-DQUACKGIS_DUCKLAKE_SOURCE={(sources / 'ducklake').resolve()}",
                f"-DQUACKGIS_SPATIAL_SOURCE={(sources / 'spatial').resolve()}",
                "-DENABLE_SANITIZER=FALSE",
                "-DENABLE_UBSAN=0",
                "-DCMAKE_C_COMPILER_LAUNCHER=",
                "-DCMAKE_CXX_COMPILER_LAUNCHER=",
            ]
        ),
        "OVERRIDE_GIT_DESCRIBE": f"v{version}-0-g{commit}",
    })
    return environment


def require_contained(path: Path, owner: Path, label: str) -> None:
    owner_resolved = owner.resolve()
    try:
        path.resolve().relative_to(owner_resolved)
        relative = path.relative_to(owner)
    except ValueError as error:
        raise ValueError(f"{label} escapes its owned directory") from error
    current = owner
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"{label} traverses a symlink: {current}")


def remove_owned_tree(path: Path, owner: Path, label: str) -> None:
    require_contained(path, owner, label)
    if path.exists():
        if not path.is_dir():
            raise ValueError(f"{label} is not a directory")
        shutil.rmtree(path)


def prepare_isolated_build_state(prepared: Path, vcpkg: Path | None = None) -> None:
    state = prepared / "build-state"
    remove_owned_tree(state, prepared, "central build state")
    for name in (
        "home",
        "tmp",
        "xdg-cache",
        "vcpkg-binary-cache",
        "vcpkg-downloads",
    ):
        (state / name).mkdir(parents=True)
    if vcpkg is not None:
        for name in ("buildtrees", "downloads", "packages"):
            remove_owned_tree(vcpkg / name, vcpkg, f"vcpkg {name}")


def reject_ignored_source_inputs(checkout: Path, allowed_prefixes: tuple[str, ...]) -> None:
    ignored = output(
        ["git", "ls-files", "--others", "--ignored", "--exclude-standard"],
        cwd=checkout,
    ).splitlines()
    unexpected = [
        path
        for path in ignored
        if not any(path == prefix or path.startswith(f"{prefix}/") for prefix in allowed_prefixes)
    ]
    if unexpected:
        raise ValueError(
            f"prepared checkout contains ignored source-side inputs: {unexpected[:5]}"
        )


def configure(bundle: dict[str, Any], prepared: Path) -> dict[str, Any]:
    prepare_native_bundle.prepare(bundle, prepared, ROOT)
    tool_paths = resolve_tool_paths()
    tools = verify_toolchain(bundle, tool_paths)
    sources = prepared / "sources"
    core = sources / "duckdb"
    local_config = core / "extension/extension_config_local.cmake"
    reject_ignored_source_inputs(
        core, ("build", ".cache", "extension/extension_config_local.cmake")
    )
    reject_ignored_source_inputs(sources / "ducklake", ())
    reject_ignored_source_inputs(sources / "spatial", ())
    if local_config.exists() or local_config.is_symlink():
        if local_config.is_symlink() or not local_config.is_file() or local_config.stat().st_size:
            raise ValueError("ignored DuckDB local extension config must not supply build input")
        local_config.unlink()
    configuration = core / "build/extension_configuration"
    if configuration.exists():
        remove_owned_tree(configuration, core, "central extension configuration output")
    prepare_isolated_build_state(sources.parent)
    run(
        [tool_paths["make"], "extension_configuration"],
        cwd=core,
        environment=make_environment(bundle, sources, tool_paths),
    )
    merged_path = configuration / "vcpkg.json"
    if not merged_path.is_file() or merged_path.is_symlink():
        raise ValueError("central DuckDB configuration did not emit merged vcpkg.json")
    merged = normalized_vcpkg_manifest(merged_path, sources / "spatial")
    merged_sha256 = native_bundle.canonical_sha256(merged)
    if merged_sha256 != bundle["build"]["merged_vcpkg_sha256"]:
        raise ValueError(
            "merged vcpkg graph drifted: "
            f"expected {bundle['build']['merged_vcpkg_sha256']}, got {merged_sha256}"
        )
    plan = {
        "schema_version": 1,
        "bundle_id": bundle["bundle_id"],
        "authority_sha256": native_bundle.authority_sha256(bundle, ROOT),
        "duckdb_commit": bundle["duckdb"]["source"]["commit"],
        "extensions": {
            name: {
                "commit": bundle["extensions"][name]["source"]["commit"],
                "result_tree": native_bundle.validate_series(bundle, name, ROOT)[
                    "result_tree"
                ],
            }
            for name in ("ducklake", "spatial")
        },
        "toolchain": tools,
        "vcpkg_commit": bundle["toolchain"]["vcpkg"]["commit"],
        "merged_vcpkg_sha256": merged_sha256,
        "merged_vcpkg": merged,
        "state": "configured",
    }
    write_plan(prepared, plan)
    return plan


def validate_vcpkg_checkout(target: Path, authority: dict[str, str]) -> None:
    if output(["git", "rev-parse", "HEAD"], cwd=target) != authority["commit"]:
        raise ValueError("prepared vcpkg checkout is not at the manifest commit")
    if output(["git", "remote", "get-url", "origin"], cwd=target) != authority["url"]:
        raise ValueError("prepared vcpkg origin drifted")
    if output(["git", "rev-parse", "--is-shallow-repository"], cwd=target) != "false":
        raise ValueError("vcpkg needs complete pinned history for version-tree resolution")
    flagged = [
        line
        for line in output(["git", "ls-files", "-v"], cwd=target).splitlines()
        if not line.startswith("H ")
    ]
    if flagged:
        raise ValueError(f"prepared vcpkg checkout contains non-default index flags: {flagged[:5]}")
    refreshed = subprocess.run(
        ["git", "update-index", "--really-refresh"],
        cwd=target,
        check=False,
        capture_output=True,
    )
    if refreshed.returncode != 0:
        raise ValueError("prepared vcpkg tracked bytes differ from its index")
    if subprocess.run(["git", "diff", "--quiet"], cwd=target, check=False).returncode != 0:
        raise ValueError("prepared vcpkg checkout contains tracked modifications")
    if output(["git", "write-tree"], cwd=target) != output(
        ["git", "rev-parse", "HEAD^{tree}"], cwd=target
    ):
        raise ValueError("prepared vcpkg index contains staged modifications")
    if output(["git", "ls-files", "--others", "--exclude-standard"], cwd=target):
        raise ValueError("prepared vcpkg checkout contains unexpected untracked files")
    ignored = output(
        ["git", "ls-files", "--others", "--ignored", "--exclude-standard"],
        cwd=target,
    ).splitlines()
    unexpected_ignored = [
        path
        for path in ignored
        if not any(path == prefix or path.startswith(f"{prefix}/") for prefix in VCPKG_GENERATED_PATHS)
    ]
    if unexpected_ignored:
        raise ValueError(
            f"prepared vcpkg checkout contains unexpected ignored files: {unexpected_ignored[:5]}"
        )


def prepare_vcpkg(
    bundle: dict[str, Any], prepared: Path, environment: dict[str, str]
) -> Path:
    authority = bundle["toolchain"]["vcpkg"]
    target = prepared / "toolchain/vcpkg"
    require_contained(target, prepared, "vcpkg toolchain checkout")
    if target.exists():
        if target.is_symlink() or not (target / ".git").is_dir():
            raise ValueError("refusing unrecognized vcpkg toolchain directory")
        if output(["git", "rev-parse", "--is-shallow-repository"], cwd=target) == "true":
            run(["git", "fetch", "--quiet", "--unshallow", "origin"], cwd=target)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--quiet", "--no-checkout", authority["url"], str(target)])
        run(["git", "checkout", "--quiet", "--detach", authority["commit"]], cwd=target)
    validate_vcpkg_checkout(target, authority)
    for name in ("buildtrees", "downloads", "packages"):
        remove_owned_tree(target / name, target, f"vcpkg {name}")
    executable = target / "vcpkg"
    if executable.exists() or executable.is_symlink():
        if executable.is_dir() and not executable.is_symlink():
            raise ValueError("prepared vcpkg executable path is not a regular file")
        executable.unlink()
    disable_metrics = target / "vcpkg.disable-metrics"
    if disable_metrics.exists() or disable_metrics.is_symlink():
        if disable_metrics.is_dir() and not disable_metrics.is_symlink():
            raise ValueError("vcpkg metrics marker path is not a regular file")
        disable_metrics.unlink()
    run(
        [str(target / "bootstrap-vcpkg.sh"), "-disableMetrics"],
        cwd=target,
        environment=environment,
    )
    if executable.is_symlink() or not executable.is_file():
        raise ValueError("vcpkg bootstrap did not emit a regular executable")
    return target


def build(bundle: dict[str, Any], prepared: Path) -> dict[str, Any]:
    plan = configure(bundle, prepared)
    sources = prepared / "sources"
    tool_paths = resolve_tool_paths()
    verify_toolchain(bundle, tool_paths)
    environment = make_environment(bundle, sources, tool_paths)
    vcpkg = prepare_vcpkg(bundle, prepared, environment)
    prepare_isolated_build_state(prepared, vcpkg)
    environment = make_environment(bundle, sources, tool_paths)
    environment.update(
        {
            "USE_MERGED_VCPKG_MANIFEST": "1",
            "VCPKG_TOOLCHAIN_PATH": str(vcpkg / "scripts/buildsystems/vcpkg.cmake"),
        }
    )
    build_root = sources / "duckdb/build/release"
    if build_root.exists():
        remove_owned_tree(build_root, sources / "duckdb", "central release build output")
    run(
        [tool_paths["make"], "release"],
        cwd=sources / "duckdb",
        environment=environment,
    )
    candidates = candidate_paths(build_root)
    for name, path in candidates.items():
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"central build did not emit {name}: {path}")
    plan["state"] = "built-unaccepted"
    plan["vcpkg_binary_sha256"] = native_bundle.file_sha256(vcpkg / "vcpkg")
    plan["candidate_artifacts"] = {
        name: native_bundle.file_sha256(path) for name, path in candidates.items()
    }
    runner = build_root / "test/unittest"
    if runner.is_symlink() or not runner.is_file():
        raise ValueError("central build did not emit the upstream test runner")
    plan["test_runner_sha256"] = native_bundle.file_sha256(runner)
    write_plan(prepared, plan)
    return plan


def candidate_paths(build_root: Path) -> dict[str, Path]:
    return {
        "duckdb": build_root / "duckdb",
        "libduckdb.so": build_root / "src/libduckdb.so",
        "ducklake.duckdb_extension": build_root / "extension/ducklake/ducklake.duckdb_extension",
        "spatial.duckdb_extension": build_root / "extension/spatial/spatial.duckdb_extension",
    }


def write_plan(prepared: Path, plan: dict[str, Any]) -> None:
    path = prepared / PLAN_NAME
    if path.is_symlink():
        raise ValueError("central candidate plan cannot be a symlink")
    partial = prepared / f".{PLAN_NAME}.partial"
    if partial.exists() or partial.is_symlink():
        raise ValueError("remove interrupted central candidate plan explicitly")
    try:
        with partial.open("x", encoding="utf-8") as output_file:
            output_file.write(json.dumps(plan, indent=2, sort_keys=True) + "\n")
        partial.replace(path)
    finally:
        if partial.exists() and not partial.is_symlink():
            partial.unlink()


def test_candidate(bundle: dict[str, Any], prepared: Path) -> dict[str, Any]:
    plan_path = prepared / PLAN_NAME
    if not plan_path.is_file() or plan_path.is_symlink():
        raise ValueError("central candidate plan is missing")
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    if plan.get("state") not in {"built-unaccepted", "upstream-tested-unaccepted"}:
        raise ValueError("central candidate must be built before tests run")
    if plan.get("authority_sha256") != native_bundle.authority_sha256(bundle, ROOT):
        raise ValueError("central candidate authority changed after build")
    prepare_native_bundle.prepare(bundle, prepared, ROOT)
    sources = prepared / "sources"
    reject_ignored_source_inputs(
        sources / "duckdb", ("build", ".cache", "extension/extension_config_local.cmake")
    )
    reject_ignored_source_inputs(sources / "ducklake", ())
    reject_ignored_source_inputs(sources / "spatial", ())
    local_config = sources / "duckdb/extension/extension_config_local.cmake"
    if local_config.exists() or local_config.is_symlink():
        if local_config.is_symlink() or not local_config.is_file() or local_config.stat().st_size:
            raise ValueError("ignored DuckDB local extension config changed after build")
    build_root = sources / "duckdb/build/release"
    candidates = candidate_paths(build_root)
    expected = plan.get("candidate_artifacts")
    if not isinstance(expected, dict):
        raise ValueError("central candidate plan has no artifact digests")
    for name, path in candidates.items():
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"central candidate artifact is missing: {name}")
        if native_bundle.file_sha256(path) != expected.get(name):
            raise ValueError(f"central candidate artifact drifted after build: {name}")
    version = output([str(candidates["duckdb"]), "--version"])
    if not version.startswith(
        f"v{bundle['duckdb']['version']} (Variegata) {bundle['duckdb']['source']['commit'][:10]}"
    ):
        raise ValueError(f"central candidate reports unexpected DuckDB version: {version}")
    test_state = prepared / "candidate-test-state"
    remove_owned_tree(test_state, prepared, "candidate test state")
    (test_state / "home").mkdir(parents=True)
    environment = {
        key: value for key, value in os.environ.items() if key in BUILD_ENVIRONMENT_KEYS
    }
    environment.update(
        {
            "HOME": str((test_state / "home").absolute()),
            "TMPDIR": str(test_state.absolute()),
            "XDG_CACHE_HOME": str((test_state / "cache").absolute()),
            "LOCAL_EXTENSION_REPO": str((build_root / "repository").absolute()),
        }
    )
    (test_state / "cache").mkdir()
    extension_paths = {
        "ducklake": candidates["ducklake.duckdb_extension"].resolve(),
        "spatial": candidates["spatial.duckdb_extension"].resolve(),
    }
    escaped = {name: str(path).replace("'", "''") for name, path in extension_paths.items()}
    run(
        [
            str(candidates["duckdb"]),
            "-unsigned",
            "-batch",
            ":memory:",
            "-c",
            "SELECT CASE WHEN count(*) = 2 AND count_if(loaded OR installed) = 0 THEN 1 "
            "ELSE error('candidate extensions are present before explicit load') END "
            "FROM duckdb_extensions() WHERE extension_name IN ('ducklake', 'spatial'); "
            "LOAD '{ducklake}'; LOAD '{spatial}'; "
            "SELECT CASE WHEN count(*) = 2 AND count_if(loaded AND NOT installed "
            "AND install_mode = 'NOT_INSTALLED') = 2 "
            "AND count_if(extension_name = 'ducklake' AND extension_version = '{ducklake_version}') = 1 "
            "AND count_if(extension_name = 'spatial' AND extension_version = '{spatial_version}') = 1 "
            "AND ST_AsText(ST_Point(1, 2)) = 'POINT (1 2)' THEN 1 "
            "ELSE error('candidate extension load mismatch') END "
            "FROM duckdb_extensions() WHERE extension_name IN ('ducklake', 'spatial');".format(
                **escaped,
                ducklake_version=bundle["extensions"]["ducklake"]["source"]["commit"][:7],
                spatial_version=bundle["extensions"]["spatial"]["source"]["commit"][:7],
            ),
        ],
        environment=environment,
    )
    runner = build_root / "test/unittest"
    if runner.is_symlink() or not runner.is_file():
        raise ValueError("central candidate upstream test runner is missing")
    if native_bundle.file_sha256(runner) != plan.get("test_runner_sha256"):
        raise ValueError("central candidate upstream test runner drifted after build")
    repository_root = (
        build_root
        / "repository"
        / f"v{bundle['duckdb']['version']}"
        / "linux_amd64"
    )
    repository_artifacts = {
        "ducklake": repository_root / "ducklake.duckdb_extension",
        "spatial": repository_root / "spatial.duckdb_extension",
    }
    for component, path in repository_artifacts.items():
        candidate = candidates[f"{component}.duckdb_extension"]
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"central candidate repository is missing {component}")
        if native_bundle.file_sha256(path) != native_bundle.file_sha256(candidate):
            raise ValueError(f"central candidate repository {component} artifact drifted")
    tests = candidate_test_filters(bundle, sources)
    results: dict[str, dict[str, int | str]] = {}
    runner_output = sources / "duckdb/duckdb_unittest_tempdir"
    if runner_output.exists() or runner_output.is_symlink():
        raise ValueError(
            f"remove pre-existing DuckDB runner output explicitly: {runner_output}"
        )
    try:
        for name, test in tests.items():
            requirement = candidate_test_requirement(name)
            text = run_output(
                [
                    str(runner),
                    "--autoloading",
                    "available",
                    "--require",
                    requirement,
                    str(test),
                ],
                environment=environment,
            )
            result = parse_test_result(text, name)
            results[name] = {
                "filter": str(test.relative_to(prepared)),
                **result,
            }
    finally:
        if runner_output.exists() or runner_output.is_symlink():
            remove_owned_tree(
                runner_output,
                sources / "duckdb",
                "DuckDB candidate test-runner output",
            )
    plan["state"] = "upstream-tested-unaccepted"
    plan["candidate_tests"] = {
        "offline_extension_load": "passed",
        "test_runner_sha256": plan["test_runner_sha256"],
        "upstream": results,
        "remaining_quackgis_groups": bundle["test_groups"]["quackgis"],
    }
    write_plan(prepared, plan)
    return plan


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=native_bundle.BUNDLE_PATH)
    parser.add_argument("--prepared", type=Path, default=DEFAULT_PREPARED)
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--build", action="store_true", help="build the pinned graph after configuration")
    action.add_argument("--test", action="store_true", help="test already-built candidate artifacts")
    args = parser.parse_args(argv)
    try:
        bundle = native_bundle.load_bundle(args.manifest.resolve(), ROOT)
        prepared = prepare_native_bundle.require_workspace_output(args.prepared)
        if args.test:
            plan = test_candidate(bundle, prepared)
        else:
            plan = build(bundle, prepared) if args.build else configure(bundle, prepared)
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"native bundle build failed: {error}", file=sys.stderr)
        return 1
    print(
        "native_bundle_build_ok "
        f"bundle={bundle['bundle_id']} state={plan['state']} "
        f"merged_vcpkg_sha256={plan['merged_vcpkg_sha256']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
