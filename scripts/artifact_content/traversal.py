def tar_entries(path: str):
    required_hooks = (
        "_fromtarfile", "_proc_pax", "_proc_gnulong", "_proc_sparse",
        "_proc_gnusparse_00", "_proc_gnusparse_01", "_proc_gnusparse_10",
    )
    missing_hook = next(
        (hook for hook in required_hooks if not callable(getattr(tarfile.TarInfo, hook, None))),
        None,
    )
    if missing_hook:
        raise Rejected(f"unsupported Python tarfile: required tarfile hook {missing_hook} is unavailable")

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
