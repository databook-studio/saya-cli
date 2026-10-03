#!/usr/bin/env python3
"""Reject unexpected files and local build data in Saya release artifacts."""

import argparse
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
SCAN_CHUNK_BYTES = 64 * 1024
SCAN_OVERLAP_BYTES = 512
MANIFEST_BYTES = 2 * 1024 * 1024
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
    rb"/private/var/folders/[^/\x00\s]{1,256}/|/(?:workspace|workspaces)/[^/\x00\s]{1,256}/|"
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
    if any(
        part.lower() in FORBIDDEN_PARTS
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


def archive_entries(path: str):
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as archive:
            items = archive.infolist()
            if len(items) > MAX_ARCHIVE_ENTRIES:
                raise Rejected("archive has too many entries")
            for item in items:
                if item.flag_bits & 1:
                    raise Rejected(f"encrypted ZIP entry: {item.filename}")
                mode = item.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise Rejected(f"symlink entry: {item.filename}")
                file_type = stat.S_IFMT(mode)
                if file_type and file_type not in (stat.S_IFREG, stat.S_IFDIR):
                    raise Rejected(f"unsafe ZIP entry type: {item.filename}")
                if item.file_size > MAX_ENTRY_BYTES:
                    raise Rejected(f"oversized archive entry: {item.filename}")
                yield item.filename, item.file_size, item.is_dir(), lambda item=item: archive.open(item),
    else:
        try:
            archive = tarfile.open(path, "r:*")
        except (tarfile.TarError, OSError) as error:
            raise Rejected(f"unsupported or unreadable archive: {error}") from error
        with archive:
            count = 0
            for item in archive:
                count += 1
                if count > MAX_ARCHIVE_ENTRIES:
                    raise Rejected("archive has too many entries")
                if item.issym() or item.islnk() or not (item.isdir() or item.isfile()):
                    raise Rejected(f"unsafe archive entry type: {item.name}")
                if item.size > MAX_ENTRY_BYTES:
                    raise Rejected(f"oversized archive entry: {item.name}")
                opener = (lambda item=item: archive.extractfile(item)) if item.isfile() else None
                yield item.name, item.size, item.isdir(), opener


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


def scan_stream(name: str, stream, binary: bool = False, keep: bool = False, max_bytes: int = MAX_ENTRY_BYTES):
    overlap = b""
    retained = bytearray() if keep else None
    total = 0
    while True:
        chunk = stream.read(min(SCAN_CHUNK_BYTES, max_bytes - total + 1))
        if not chunk:
            break
        total += len(chunk)
        if total > min(MAX_ENTRY_BYTES, max_bytes):
            raise Rejected(f"oversized archive entry: {name}")
        overlap = content_check(name, chunk, binary, overlap)
        if retained is not None:
            if total > MANIFEST_BYTES:
                raise Rejected("crate Cargo.toml exceeds manifest size limit")
            retained.extend(chunk)
    return (bytes(retained) if retained is not None else None), total


def scan_archive(path: str, kind: str) -> None:
    root = expected_root(path, kind)
    headers = []
    header_names = set()
    header_bytes = 0
    executable_names = []
    for raw_name, declared_size, is_dir, _opener in archive_entries(path):
        name = canonical(raw_name)
        if name in header_names:
            raise Rejected(f"duplicate archive path: {name}")
        header_names.add(name)
        if len(headers) >= MAX_ARCHIVE_ENTRIES:
            raise Rejected("archive has too many entries")
        if declared_size > MAX_ENTRY_BYTES:
            raise Rejected(f"oversized archive entry: {name}")
        header_bytes += declared_size
        if header_bytes > MAX_ARCHIVE_BYTES:
            raise Rejected("archive payload exceeds the size limit")
        headers.append((name, declared_size, is_dir))
        if kind == "archive" and name in {f"{root}/saya", f"{root}/saya.exe"}:
            executable_names.append(name)
    if kind == "archive" and len(executable_names) != 1:
        raise Rejected("release archive must contain exactly one executable")

    entries = {}
    total_bytes = 0
    observed_archive_bytes = 0
    manifest = None
    source_file = False
    for (raw_name, declared_size, is_dir), (_header_name, _header_size, _header_dir, opener) in zip(
        ((name, size, directory) for name, size, directory in headers), archive_entries(path), strict=True
    ):
        # Re-read headers in lockstep after budget/cardinality validation; only
        # payload streams are opened in this pass.
        actual_name, actual_size, actual_dir = _header_name, _header_size, _header_dir
        if (raw_name, declared_size, is_dir) != (actual_name, actual_size, actual_dir):
            raise Rejected("archive headers changed during scan")
        name = canonical(raw_name)
        if name in entries:
            raise Rejected(f"duplicate archive path: {name}")
        if len(entries) >= MAX_ARCHIVE_ENTRIES:
            raise Rejected("archive has too many entries")
        if declared_size > MAX_ENTRY_BYTES:
            raise Rejected(f"oversized archive entry: {name}")
        total_bytes += declared_size
        if total_bytes > MAX_ARCHIVE_BYTES:
            raise Rejected("archive payload exceeds the size limit")
        entries[name] = (is_dir, declared_size)
        if kind == "crate" and name.startswith(f"{root}/src/") and not is_dir:
            source_file = True
        if is_dir:
            if declared_size != 0:
                raise Rejected(f"directory entry has payload: {name}")
            continue
        if opener is None:
            raise Rejected(f"unreadable archive entry: {name}")
        with opener() as stream:
            retained, observed_size = scan_stream(
                name,
                stream,
                binary=(kind == "archive" and name in {f"{root}/saya", f"{root}/saya.exe"}),
                keep=(kind == "crate" and name == f"{root}/Cargo.toml"),
                max_bytes=MAX_ARCHIVE_BYTES - observed_archive_bytes,
            )
        observed_archive_bytes += observed_size
        if observed_size != declared_size:
            raise Rejected(f"archive entry size mismatch: {name}")
        if name == f"{root}/Cargo.toml":
            manifest = retained
    if not entries:
        raise Rejected("empty archive")
    if kind == "archive":
        check_release_archive(root, entries, executable_names)
    else:
        check_crate(root, entries, manifest, source_file)


def check_release_archive(root: str, entries: dict, executable_names: list[str]) -> None:
    allowed = {f"{root}/{name}" for name in ("saya", "saya.exe", "README.md", "LICENSE", "SECURITY.md")}
    for name, (is_dir, _) in entries.items():
        if name == root and is_dir:
            continue
        if name not in allowed or is_dir:
            raise Rejected(f"unexpected release archive entry: {name}")
    if len(executable_names) != 1:
        raise Rejected("release archive must contain exactly one executable")
    names = set(entries)
    required = {executable_names[0], f"{root}/README.md", f"{root}/LICENSE", f"{root}/SECURITY.md"}
    missing = required - names
    if missing:
        raise Rejected(f"release archive missing required entries: {', '.join(sorted(missing))}")


def check_crate(root: str, entries: dict, manifest: bytes | None, source_file: bool) -> None:
    file_roots = {
        "Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json",
        "build.rs", "README.md",
    }
    directory_roots = {"src", "tests", "examples", "benches", "proptest-regressions"}
    for name, (is_dir, _) in entries.items():
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
    cargo_toml = f"{root}/Cargo.toml"
    if cargo_toml not in entries or entries[cargo_toml][0]:
        raise Rejected("crate payload missing regular Cargo.toml")
    if not source_file:
        raise Rejected("crate payload missing regular source file")
    if manifest is None or not re.search(rb"(?m)^license(?:-file)?\s*=\s*\"[^\"]+\"", manifest):
        raise Rejected("crate Cargo.toml missing license or license-file metadata")


def scan_binary(path: str) -> None:
    try:
        if os.path.getsize(path) > MAX_ENTRY_BYTES:
            raise Rejected("binary exceeds the size limit")
        with open(path, "rb") as binary:
            scan_stream(os.path.basename(path), binary, binary=True)
    except OSError as error:
        raise Rejected(f"cannot read binary: {error}") from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("archive", "crate", "binary"))
    parser.add_argument("path")
    args = parser.parse_args()
    try:
        scan_binary(args.path) if args.kind == "binary" else scan_archive(args.path, args.kind)
    except (Rejected, OSError, tarfile.TarError, zipfile.BadZipFile, RuntimeError, EOFError, zlib.error) as error:
        print(f"artifact content check failed: {error}", file=sys.stderr)
        return 1
    print(f"artifact content check passed: {args.kind} {args.path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
