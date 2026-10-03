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


def tar_entries(path: str):
    class MetadataBudget:
        def __init__(self):
            self.bytes = 0
            self.headers = 0

        def header(self) -> None:
            self.headers += 1
            if self.headers > MAX_ARCHIVE_ENTRIES:
                raise Rejected("archive has too many tar headers")

        def metadata(self, size: int) -> None:
            if size < 0 or size > MAX_TAR_METADATA_BYTES:
                raise Rejected("tar metadata size limit exceeded")
            self.bytes += size
            if self.bytes > MAX_ARCHIVE_METADATA_BYTES:
                raise Rejected("tar aggregate metadata size limit exceeded")

    budget = MetadataBudget()

    class BoundedTarInfo(tarfile.TarInfo):
        @classmethod
        def _fromtarfile(cls, archive, *, dircheck=True):
            budget.header()
            return super()._fromtarfile(archive, dircheck=dircheck)

        def _proc_pax(self, archive):
            budget.metadata(self.size)
            return super()._proc_pax(archive)

        def _proc_gnulong(self, archive):
            budget.metadata(self.size)
            return super()._proc_gnulong(archive)

        def _proc_sparse(self, archive):
            raise Rejected("GNU sparse TAR entries are not supported")

        def _proc_gnusparse_00(self, next_item, raw_headers):
            raise Rejected("GNU PAX sparse entries are not supported")

        def _proc_gnusparse_01(self, next_item, pax_headers):
            raise Rejected("GNU PAX sparse entries are not supported")

        def _proc_gnusparse_10(self, next_item, pax_headers, archive):
            raise Rejected("GNU PAX sparse entries are not supported")

    with open(path, "rb") as raw:
        with gzip.GzipFile(fileobj=raw) as compressed:
            expanded = ExpandedReader(compressed)
            try:
                with tarfile.open(fileobj=expanded, mode="r|", tarinfo=BoundedTarInfo) as archive:
                    total_declared = 0
                    entries = 0
                    for item in archive:
                        entries += 1
                        if entries > MAX_ARCHIVE_ENTRIES:
                            raise Rejected("archive has too many entries")
                        if item.issym() or item.islnk() or not (item.isdir() or item.isfile()):
                            raise Rejected(f"unsafe archive entry type: {item.name}")
                        if item.size > MAX_ENTRY_BYTES:
                            raise Rejected(f"oversized archive entry: {item.name}")
                        total_declared += item.size
                        if total_declared > MAX_ARCHIVE_BYTES:
                            raise Rejected("archive payload exceeds the size limit")
                        opener = (lambda item=item: archive.extractfile(item)) if item.isfile() else None
                        yield item.name, item.size, item.isdir(), opener
            finally:
                expanded.close()


def zip_entries(path: str, kind: str):
    with open(path, "rb") as raw:
        # The stdlib footer reader resolves ZIP64 too, using only its bounded
        # end-record/comment window; inspect count/size before ZipFile builds
        # the central-directory byte buffer and ZipInfo table.
        end_record = zipfile._EndRecData(raw)
        if end_record is None:
            raise zipfile.BadZipFile("file is not a ZIP archive")
        count = end_record[zipfile._ECD_ENTRIES_TOTAL]
        central_size = end_record[zipfile._ECD_SIZE]
        if count > MAX_ARCHIVE_ENTRIES:
            raise Rejected("archive has too many entries")
        if central_size > MAX_ZIP_METADATA_BYTES:
            raise Rejected("ZIP metadata size limit exceeded")
        raw.seek(0)
        with zipfile.ZipFile(raw) as archive:
            items = archive.infolist()
            if len(items) != count:
                raise Rejected("ZIP central directory entry count mismatch")
            names = set()
            total_declared = 0
            executable_names = []
            root = expected_root(path, kind)
            for item in items:
                name = canonical(item.filename)
                if name in names:
                    raise Rejected(f"duplicate archive path: {name}")
                names.add(name)
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
                total_declared += item.file_size
                if total_declared > MAX_ARCHIVE_BYTES:
                    raise Rejected("archive payload exceeds the size limit")
                if kind == "archive" and name in {f"{root}/saya", f"{root}/saya.exe"}:
                    executable_names.append(name)
            if kind == "archive" and len(executable_names) != 1:
                raise Rejected("release archive must contain exactly one executable")
            for item in items:
                yield item.filename, item.file_size, item.is_dir(), lambda item=item: archive.open(item)


def archive_entries(path: str, kind: str):
    if kind == "archive" and path.endswith(".zip"):
        yield from zip_entries(path, kind)
    else:
        yield from tar_entries(path)


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
    ensure_archive_input(path)
    executable_names = []
    entries = {}
    total_bytes = 0
    observed_archive_bytes = 0
    manifest = None
    source_file = False
    for raw_name, declared_size, is_dir, opener in archive_entries(path, kind):
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
        if kind == "archive" and name in {f"{root}/saya", f"{root}/saya.exe"}:
            executable_names.append(name)
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
    except (
        Rejected,
        OSError,
        ValueError,
        tarfile.TarError,
        zipfile.BadZipFile,
        RuntimeError,
        EOFError,
        zlib.error,
    ) as error:
        print(f"artifact content check failed: {error}", file=sys.stderr)
        return 1
    print(f"artifact content check passed: {args.kind} {args.path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
