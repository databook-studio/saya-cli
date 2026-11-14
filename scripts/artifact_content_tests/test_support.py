#!/usr/bin/env python3
"""Behavior tests for the release artifact content gate."""

import importlib.util
import io
import os
import shlex
import stat
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
GATE = ROOT / "scripts" / "check-artifact-content.py"
SENTINEL = b"SAYA_ARTIFACT_FORBIDDEN_SENTINEL"
SPEC = importlib.util.spec_from_file_location("artifact_gate", GATE)
GATE_MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(GATE_MODULE)


def tar_gz(path: Path, entries: dict[str, bytes | None], symlink: str | None = None) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for name, content in entries.items():
            info = tarfile.TarInfo(name)
            if content is None:
                info.type = tarfile.DIRTYPE
                archive.addfile(info)
            else:
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
        if symlink:
            info = tarfile.TarInfo(symlink)
            info.type = tarfile.SYMTYPE
            info.linkname = "saya"
            archive.addfile(info)


def zip_file(path: Path, entries: dict[str, bytes | None]) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        for name, content in entries.items():
            archive.writestr(name, b"" if content is None else content)


def valid_crate(path: Path, extra: dict[str, bytes | None] | None = None) -> None:
    root = path.name.removesuffix(".crate")
    entries: dict[str, bytes | None] = {
        f"{root}/Cargo.toml": b'[package]\nlicense = "Apache-2.0"',
        f"{root}/src/lib.rs": b"pub fn public_api() {}",
    }
    if extra:
        entries.update({f"{root}/{name}": value for name, value in extra.items()})
    tar_gz(path, entries)


def sparse_crate(path: Path, *, pax_parser: str | None) -> None:
    root = path.name.removesuffix(".crate")
    with tarfile.open(
        path,
        "w:gz",
        format=tarfile.PAX_FORMAT if pax_parser else tarfile.GNU_FORMAT,
    ) as archive:
        for name, content in (("Cargo.toml", b'[package]\nlicense = "Apache-2.0"'), ("LICENSE", b"license")):
            info = tarfile.TarInfo(f"{root}/{name}")
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))
        info = tarfile.TarInfo(f"{root}/src/lib.rs")
        info.size = 1
        if pax_parser:
            info.pax_headers = {
                "_proc_gnusparse_00": {"GNU.sparse.size": "1"},
                "_proc_gnusparse_01": {"GNU.sparse.map": "0,1"},
                "_proc_gnusparse_10": {"GNU.sparse.major": "1", "GNU.sparse.minor": "0"},
            }[pax_parser]
            archive.addfile(info, io.BytesIO(b"x"))
            return
        archive.addfile(info, io.BytesIO(b"x"))

    if not pax_parser:
        import gzip

        with gzip.open(path, "rb") as compressed:
            payload = bytearray(compressed.read())
        header = payload.index(f"{root}/src/lib.rs".encode()) // 512 * 512
        payload[header + 148 : header + 156] = b"        "
        payload[header + 156] = ord("S")
        checksum = sum(payload[header : header + 512])
        payload[header + 148 : header + 156] = f"{checksum:06o}\0 ".encode()
        with gzip.open(path, "wb") as compressed:
            compressed.write(payload)
