#!/usr/bin/env python3
"""Reject unexpected files and local build data in Saya release artifacts."""

import argparse
import gzip
import os
import re
import stat
import sys
import tarfile
import zipfile
import zlib
from pathlib import PurePosixPath

SENTINEL = b"SAYA_ARTIFACT_FORBIDDEN_SENTINEL"
MAX_ARCHIVE_ENTRIES = 20_000
MAX_ENTRY_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_ARCHIVE_INPUT_BYTES = 1024 * 1024 * 1024
MAX_ZIP_METADATA_BYTES = 16 * 1024 * 1024
MAX_TAR_METADATA_BYTES = 1024 * 1024
MAX_ARCHIVE_METADATA_BYTES = 16 * 1024 * 1024
SCAN_CHUNK_BYTES = 64 * 1024
SCAN_OVERLAP_BYTES = 512
MANIFEST_BYTES = 2 * 1024 * 1024
MAX_ARCHIVE_EXPANDED_BYTES = (
    MAX_ARCHIVE_BYTES + MAX_ARCHIVE_METADATA_BYTES + MAX_ARCHIVE_ENTRIES * 1024 + 1024
)
FORBIDDEN_PARTS = {
    ".aws", ".cargo", ".claude", ".codex", ".cursor", ".git", ".saya",
    ".ssh", "__pycache__", "auth.json", "credentials", "secrets",
    "secrets.toml", "sessions", "token", "token.json",
}
FORBIDDEN_NAMES = {"config.toml", "connections.toml", "id_rsa", "id_ed25519"}
CREDENTIAL_EXTENSIONS = {".json", ".toml", ".yaml", ".yml", ".ini", ".txt", ".key", ".pem", ".p8", ".p12", ".pfx"}
CREDENTIAL_STEM = re.compile(r"(?:^|[._-])(credential|credentials|secret|secrets|token|auth)(?:$|[._-])")
LOCAL_PATH = re.compile(
    rb"(?:/Users/[^/\x00\s]{1,256}/|/home/[^/\x00\s]{1,256}/|"
    rb"/private/var/folders/[^/\x00\s]{1,256}/|"
    rb"(?<![A-Za-z0-9_./-])/(?:workspace|workspaces)/[^/\x00\s]{1,256}/|"
    rb"[A-Za-z]:\\Users\\[^\\\x00\s]{1,256}\\|"
    rb"[A-Za-z]:\\a\\[^\\\x00\s]{1,256}\\|[A-Za-z]:\\workspace\\)",
    re.IGNORECASE,
)


class Rejected(ValueError):
    pass


def canonical(name: str) -> str:
    if "\\" in name or ":" in name or "\x00" in name or name.startswith("/"):
        raise Rejected(f"unsafe entry path: {name!r}")
    normalized = name[:-1] if name.endswith("/") else name
    path = PurePosixPath(normalized)
    if not normalized or any(part in ("", ".", "..") for part in normalized.split("/")):
        raise Rejected(f"unsafe entry path: {name!r}")
    if path.is_absolute() or str(path) != normalized:
        raise Rejected(f"noncanonical entry path: {name!r}")
    parts = path.parts
    session_test_source = (
        len(parts) == 4
        and parts[1:3] == ("tests", "sessions")
        and parts[3].endswith(".rs")
        and not any(part.startswith(".") for part in parts)
        and not CREDENTIAL_STEM.search(os.path.splitext(parts[3].lower())[0])
    )
    if any(
        (
            part.lower() in FORBIDDEN_PARTS
            and not (part.lower() == "sessions" and session_test_source)
        )
        or part.lower() in FORBIDDEN_NAMES
        or part.lower().startswith(".env")
        for part in parts
    ):
        raise Rejected(f"forbidden payload path: {name}")
    for part in parts:
        lower = part.lower()
        base, extension = os.path.splitext(lower)
        if CREDENTIAL_STEM.search(base) and extension in CREDENTIAL_EXTENSIONS:
            raise Rejected(f"credential-like payload path: {name}")
    if any(part.lower().endswith((".pem", ".key", ".p8", ".p12", ".pfx")) for part in parts):
        raise Rejected(f"credential-like payload path: {name}")
    return name.rstrip("/")


def content_check(name: str, data: bytes, binary: bool = False, overlap: bytes = b"") -> bytes:
    scan = overlap + data
    if SENTINEL in scan:
        raise Rejected(f"forbidden sentinel in {name}")
    if binary and LOCAL_PATH.search(scan):
        raise Rejected(f"local home/workspace path in binary {name}")
    return scan[-SCAN_OVERLAP_BYTES:]


def ensure_archive_input(path: str) -> None:
    try:
        size = os.path.getsize(path)
    except OSError as error:
        raise Rejected(f"cannot read archive input: {error}") from error
    if size > MAX_ARCHIVE_INPUT_BYTES:
        raise Rejected("archive input size limit exceeded")


class ExpandedReader:
    """Bound every decompressed read before it can allocate an oversized result."""

    def __init__(self, stream):
        self.stream = stream
        self.total = 0

    def read(self, size: int = -1) -> bytes:
        if size < 0 or size > SCAN_CHUNK_BYTES:
            raise Rejected("archive expanded read request exceeds the chunk limit")
        remaining = MAX_ARCHIVE_EXPANDED_BYTES - self.total
        data = self.stream.read(min(size, remaining + 1))
        if len(data) > remaining:
            raise Rejected("archive expanded data exceeds the size limit")
        self.total += len(data)
        return data

    def tell(self) -> int:
        return self.stream.tell()

    def close(self) -> None:
        self.stream.close()
