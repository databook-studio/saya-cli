#!/usr/bin/env python3
"""Reject unexpected files and local build data in Saya release artifacts."""

import argparse
import os
import re
import stat
import sys
import tarfile
import zipfile
from pathlib import PurePosixPath

SENTINEL = b"SAYA_ARTIFACT_FORBIDDEN_SENTINEL"
MAX_ARCHIVE_ENTRIES = 20_000
MAX_ENTRY_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
FORBIDDEN_PARTS = {
    ".aws", ".cargo", ".claude", ".codex", ".cursor", ".git", ".saya",
    ".ssh", "__pycache__", "auth.json", "credentials", "secrets",
    "secrets.toml", "sessions", "token", "token.json",
}
FORBIDDEN_NAMES = {"config.toml", "connections.toml", "id_rsa", "id_ed25519"}
LOCAL_PATH = re.compile(
    rb"(?:/Users/[^/\x00\s]+/|/home/[^/\x00\s]+/|"
    rb"/private/var/folders/[^/\x00\s]+/|/(?:workspace|workspaces)/[^/\x00\s]+/|"
    rb"[A-Za-z]:\\Users\\[^\\\x00\s]+\\|"
    rb"[A-Za-z]:\\a\\[^\\\x00\s]+\\|[A-Za-z]:\\workspace\\)",
    re.IGNORECASE,
)


class Rejected(ValueError):
    pass


def canonical(name: str) -> str:
    if "\\" in name or "\x00" in name or name.startswith("/"):
        raise Rejected(f"unsafe entry path: {name!r}")
    normalized = name[:-1] if name.endswith("/") else name
    path = PurePosixPath(normalized)
    if not normalized or any(part in ("", ".", "..") for part in normalized.split("/")):
        raise Rejected(f"unsafe entry path: {name!r}")
    if path.is_absolute() or str(path) != normalized:
        raise Rejected(f"noncanonical entry path: {name!r}")
    parts = path.parts
    if any(
        part.lower() in FORBIDDEN_PARTS
        or part.lower() in FORBIDDEN_NAMES
        or part.lower().startswith(".env")
        for part in parts
    ):
        raise Rejected(f"forbidden payload path: {name}")
    if any(part.lower().endswith((".pem", ".key", ".p8")) for part in parts):
        raise Rejected(f"credential-like payload path: {name}")
    return name.rstrip("/")


def content_check(name: str, data: bytes, binary: bool = False) -> None:
    if SENTINEL in data:
        raise Rejected(f"forbidden sentinel in {name}")
    if binary and LOCAL_PATH.search(data):
        raise Rejected(f"local home/workspace path in binary {name}")


def archive_entries(path: str):
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as archive:
            for item in archive.infolist():
                if item.file_size > MAX_ENTRY_BYTES:
                    raise Rejected(f"oversized archive entry: {item.filename}")
                mode = item.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise Rejected(f"symlink entry: {item.filename}")
                file_type = stat.S_IFMT(mode)
                if file_type and file_type not in (stat.S_IFREG, stat.S_IFDIR):
                    raise Rejected(f"unsafe ZIP entry type: {item.filename}")
                if item.is_dir():
                    yield item.filename, None, True
                else:
                    yield item.filename, archive.read(item), False
    else:
        try:
            archive = tarfile.open(path, "r:*")
        except (tarfile.TarError, OSError) as error:
            raise Rejected(f"unsupported or unreadable archive: {error}") from error
        with archive:
            for item in archive:
                if item.issym() or item.islnk() or not (item.isdir() or item.isfile()):
                    raise Rejected(f"unsafe archive entry type: {item.name}")
                if item.size > MAX_ENTRY_BYTES:
                    raise Rejected(f"oversized archive entry: {item.name}")
                if item.isdir():
                    yield item.name, None, True
                else:
                    stream = archive.extractfile(item)
                    if stream is None:
                        raise Rejected(f"unreadable archive entry: {item.name}")
                    yield item.name, stream.read(), False


def expected_root(path: str, kind: str) -> str:
    name = os.path.basename(path)
    if kind == "archive":
        for suffix in (".tar.gz", ".zip"):
            if name.endswith(suffix):
                return name[: -len(suffix)]
        raise Rejected("archive name must end in .tar.gz or .zip")
    stem = name.removesuffix(".crate")
    match = re.fullmatch(r"([A-Za-z0-9_-]+)-([0-9][A-Za-z0-9.+-]*)", stem)
    if not match:
        raise Rejected("crate name must be <package>-<version>.crate")
    return stem


def scan_archive(path: str, kind: str) -> None:
    root = expected_root(path, kind)
    entries = {}
    total_bytes = 0
    for raw_name, data, is_dir in archive_entries(path):
        if len(entries) >= MAX_ARCHIVE_ENTRIES:
            raise Rejected("archive has too many entries")
        name = canonical(raw_name)
        if name in entries:
            raise Rejected(f"duplicate archive path: {name}")
        total_bytes += len(data) if data is not None else 0
        if total_bytes > MAX_ARCHIVE_BYTES:
            raise Rejected("archive payload exceeds the size limit")
        entries[name] = (data, is_dir)
        if data is not None:
            content_check(name, data)
    if not entries:
        raise Rejected("empty archive")
    if kind == "archive":
        check_release_archive(root, entries)
    else:
        check_crate(root, entries)


def check_release_archive(root: str, entries: dict) -> None:
    allowed = {f"{root}/{name}" for name in ("saya", "saya.exe", "README.md", "LICENSE", "SECURITY.md")}
    for name, (_, is_dir) in entries.items():
        if name == root and is_dir:
            continue
        if name not in allowed or is_dir:
            raise Rejected(f"unexpected release archive entry: {name}")
    names = set(entries)
    binary_name = f"{root}/saya.exe" if f"{root}/saya.exe" in names else f"{root}/saya"
    required = {binary_name, f"{root}/README.md", f"{root}/LICENSE", f"{root}/SECURITY.md"}
    missing = required - names
    if missing:
        raise Rejected(f"release archive missing required entries: {', '.join(sorted(missing))}")
    content_check(binary_name, entries[binary_name][0], binary=True)


def check_crate(root: str, entries: dict) -> None:
    file_roots = {
        "Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json",
        "build.rs", "README.md",
    }
    directory_roots = {"src", "tests", "examples", "benches", "proptest-regressions"}
    for name, (_, is_dir) in entries.items():
        parts = name.split("/")
        if parts[0] != root:
            raise Rejected(f"crate entry outside canonical root {root}: {name}")
        if len(parts) == 1:
            if not is_dir:
                raise Rejected(f"unexpected file at crate root: {name}")
            continue
        relative = parts[1:]
        top = relative[0]
        license_file = top == "LICENSE" or top.startswith("LICENSE-") or top.startswith("COPYING")
        if top in file_roots or license_file:
            if len(relative) != 1 or is_dir:
                raise Rejected(f"unexpected crate payload path: {name}")
            continue
        if top not in directory_roots:
            raise Rejected(f"unexpected crate payload path: {name}")
        if len(relative) == 1 and not is_dir:
            raise Rejected(f"unexpected crate payload file: {name}")
    if f"{root}/Cargo.toml" not in entries:
        raise Rejected("crate payload missing Cargo.toml")
    if not any(name.startswith(f"{root}/src/") for name in entries):
        raise Rejected("crate payload missing src/")
    manifest = entries[f"{root}/Cargo.toml"][0]
    if not re.search(rb"(?m)^license(?:-file)?\s*=\s*\"[^\"]+\"", manifest):
        raise Rejected("crate Cargo.toml missing license or license-file metadata")


def scan_binary(path: str) -> None:
    try:
        if os.path.getsize(path) > MAX_ENTRY_BYTES:
            raise Rejected("binary exceeds the size limit")
        with open(path, "rb") as binary:
            content_check(os.path.basename(path), binary.read(), binary=True)
    except OSError as error:
        raise Rejected(f"cannot read binary: {error}") from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("archive", "crate", "binary"))
    parser.add_argument("path")
    args = parser.parse_args()
    try:
        scan_binary(args.path) if args.kind == "binary" else scan_archive(args.path, args.kind)
    except (Rejected, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        print(f"artifact content check failed: {error}", file=sys.stderr)
        return 1
    print(f"artifact content check passed: {args.kind} {args.path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
