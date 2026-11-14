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
