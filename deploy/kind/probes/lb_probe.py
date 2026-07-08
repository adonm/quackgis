# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import os
import socket
import struct
import sys
from collections import Counter

from probe_common import quackgis_host, quackgis_port, require


def int_env(name: str, default: int) -> int:
    value = int(os.environ.get(name, str(default)))
    require(value > 0, f"{name} must be positive")
    return value


def query_instance_id() -> str:
    rows = simple_query_rows("SELECT quackgis_instance_id()")
    require(len(rows) == 1 and len(rows[0]) == 1 and rows[0][0], f"unexpected instance rows {rows!r}")
    return str(rows[0][0])


def simple_query_rows(sql: str) -> list[list[str | None]]:
    with socket.create_connection((quackgis_host(), quackgis_port()), timeout=30) as sock:
        sock.settimeout(60)
        startup_params = (
            b"user\0postgres\0database\0quackgis\0client_encoding\0UTF8\0"
            b"application_name\0lb_probe\0\0"
        )
        sock.sendall(struct.pack("!II", len(startup_params) + 8, 196608) + startup_params)
        read_until_ready(sock)

        payload = sql.encode("utf-8") + b"\0"
        sock.sendall(b"Q" + struct.pack("!I", len(payload) + 4) + payload)
        rows: list[list[str | None]] = []
        while True:
            message_type, message = read_pg_message(sock)
            if message_type == b"D":
                rows.append(parse_data_row(message))
            elif message_type == b"E":
                raise RuntimeError(pg_error_message(message))
            elif message_type == b"Z":
                return rows


def read_until_ready(sock: socket.socket) -> None:
    while True:
        message_type, message = read_pg_message(sock)
        if message_type == b"R":
            auth_code = struct.unpack("!I", message[:4])[0]
            require(auth_code == 0, f"unsupported PostgreSQL auth request {auth_code}")
        elif message_type == b"E":
            raise RuntimeError(pg_error_message(message))
        elif message_type == b"Z":
            return


def read_pg_message(sock: socket.socket) -> tuple[bytes, bytes]:
    message_type = read_exact(sock, 1)
    length = struct.unpack("!I", read_exact(sock, 4))[0]
    require(length >= 4, f"invalid PostgreSQL message length {length}")
    return message_type, read_exact(sock, length - 4)


def read_exact(sock: socket.socket, length: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < length:
        chunk = sock.recv(length - len(chunks))
        if not chunk:
            raise RuntimeError("PostgreSQL connection closed unexpectedly")
        chunks.extend(chunk)
    return bytes(chunks)


def parse_data_row(message: bytes) -> list[str | None]:
    field_count = struct.unpack("!H", message[:2])[0]
    offset = 2
    row: list[str | None] = []
    for _ in range(field_count):
        field_len = struct.unpack("!i", message[offset : offset + 4])[0]
        offset += 4
        if field_len < 0:
            row.append(None)
            continue
        value = message[offset : offset + field_len]
        offset += field_len
        row.append(value.decode("utf-8", errors="replace"))
    return row


def pg_error_message(message: bytes) -> str:
    fields = []
    offset = 0
    while offset < len(message) and message[offset] != 0:
        code = chr(message[offset])
        offset += 1
        end = message.find(b"\0", offset)
        if end < 0:
            break
        value = message[offset:end].decode("utf-8", errors="replace")
        if code in ("S", "C", "M", "D", "H"):
            fields.append(value)
        offset = end + 1
    return ": ".join(fields) or "PostgreSQL error"


def main() -> int:
    connections = int_env("LB_CONNECTIONS", 40)
    min_instances = int_env("LB_MIN_INSTANCES", 2)
    counts = Counter(query_instance_id() for _ in range(connections))
    instances = sorted(counts)
    require(len(instances) >= min_instances, f"expected >= {min_instances} backend pods, got {counts!r}")
    print("lb_instances", ",".join(instances))
    print("lb_counts", ",".join(f"{name}:{counts[name]}" for name in instances))
    print("lb_ok", True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
