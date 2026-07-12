#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import importlib.util
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
        for name in ["tls.crt", "tls.key", "ca.crt"]:
            (tls / name).write_text(name, encoding="utf-8")
        password = root / "password"
        password.write_text("secret", encoding="utf-8")
        output = root / "rendered"
        kind_render.render(
            Namespace(
                runtime_image=digest,
                client_image=digest.replace("image", "clients"),
                tls_dir=tls,
                password_file=password,
                out_dir=output,
            )
        )
        rendered = (output / "core.yaml").read_text(encoding="utf-8")
        clients = (output / "clients.yaml").read_text(encoding="utf-8")
        assert "@@" not in rendered + clients
        assert digest in rendered
        assert "c2VjcmV0" in rendered
        assert "sslmode=verify-full" in clients
    print("kind_render_test_ok")


if __name__ == "__main__":
    main()
