from .test_support import *


class ArchiveContentCases:
    def test_tar_extension_metadata_limit_precedes_tarfile_payload_allocation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya-0.4.2-linux.tar.gz"
            with tarfile.open(path, "w:gz", format=tarfile.PAX_FORMAT) as archive:
                info = tarfile.TarInfo("saya-0.4.2-linux/" + "long-name-" * 16)
                info.size = 1
                archive.addfile(info, io.BytesIO(b"x"))
            with mock.patch.object(GATE_MODULE, "MAX_TAR_METADATA_BYTES", 32, create=True):
                with mock.patch.object(tarfile.TarInfo, "_proc_pax", side_effect=AssertionError("PAX body allocated")):
                    with self.assertRaisesRegex(GATE_MODULE.Rejected, "tar metadata size limit"):
                        GATE_MODULE.scan_archive(str(path), "archive")
    def test_tar_expanded_read_limit_bounds_stream_work(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya-0.4.2-linux.tar.gz"
            prefix = "saya-0.4.2-linux"
            tar_gz(path, {
                f"{prefix}/saya": b"binary payload",
                f"{prefix}/README.md": b"docs",
                f"{prefix}/LICENSE": b"license",
                f"{prefix}/SECURITY.md": b"policy",
            })
            with mock.patch.object(GATE_MODULE, "MAX_ARCHIVE_EXPANDED_BYTES", 512):
                with self.assertRaisesRegex(GATE_MODULE.Rejected, "expanded data exceeds"):
                    GATE_MODULE.scan_archive(str(path), "archive")
    def test_binary_streaming_detects_markers_across_chunks_without_unbounded_reads(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya"
            chunk_size = GATE_MODULE.SCAN_CHUNK_BYTES
            data = b"x" * (chunk_size - 8) + b"/Users/builduser/private/src/main.rs"
            path.write_bytes(data)
            original_open = open
            requested: list[int] = []

            class BoundedReader(io.BytesIO):
                def read(self, size=-1):
                    requested.append(size)
                    if size < 0 or size > chunk_size:
                        raise AssertionError("binary scan requested an unbounded read")
                    return super().read(size)

            with mock.patch("builtins.open", side_effect=lambda name, mode="r": BoundedReader(data) if str(name) == str(path) else original_open(name, mode)):
                with self.assertRaisesRegex(GATE_MODULE.Rejected, "local home/workspace path"):
                    GATE_MODULE.scan_binary(str(path))
            self.assertGreater(len(requested), 1)
            self.assertTrue(all(0 < size <= chunk_size for size in requested))
            path.write_bytes(b"x" * (chunk_size - 5) + SENTINEL)
            with self.assertRaisesRegex(GATE_MODULE.Rejected, "forbidden sentinel"):
                GATE_MODULE.scan_binary(str(path))
    def test_forbidden_file_and_marker_are_rejected_from_archive_and_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "saya-0.4.2-x86_64-unknown-linux-gnu.tar.gz"
            prefix = "saya-0.4.2-x86_64-unknown-linux-gnu"
            tar_gz(
                archive,
                {
                    f"{prefix}/saya": b"public binary",
                    f"{prefix}/README.md": b"docs",
                    f"{prefix}/LICENSE": b"license",
                    f"{prefix}/SECURITY.md": b"policy",
                    f"{prefix}/.env": b"private",
                },
            )
            self.assert_rejected("archive", archive)
            marked = root / "saya-0.4.2-aarch64-apple-darwin.tar.gz"
            marked_root = "saya-0.4.2-aarch64-apple-darwin"
            tar_gz(
                marked,
                {
                    f"{marked_root}/saya": b"public binary",
                    f"{marked_root}/README.md": SENTINEL,
                    f"{marked_root}/LICENSE": b"license",
                    f"{marked_root}/SECURITY.md": b"security policy",
                },
            )
            result = self.check("archive", marked)
            self.assert_rejected("archive", marked)
            self.assertIn("forbidden sentinel", result.stderr)
            clean = root / "saya-agent-0.4.2.crate"
            tar_gz(clean, {"saya-agent-0.4.2/src/lib.rs": SENTINEL})
            self.assert_rejected("crate", clean)
    def test_binary_marker_and_local_workspace_path_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "saya"
            binary.write_bytes(b"ELF data " + SENTINEL)
            self.assert_rejected("binary", binary)
            binary.write_bytes(b"ELF /Users/builduser/Projects/private-worktree/src/main.rs")
            self.assert_rejected("binary", binary)
            binary.write_bytes(rb"MZ D:\a\private-worktree\src\main.rs")
            self.assert_rejected("binary", binary)
            for path in (b"/workspace/project/src/main.rs", b"/workspaces/project/src/main.rs"):
                binary.write_bytes(b"ELF " + path)
                self.assert_rejected("binary", binary)
    def test_binary_scan_accepts_repository_relative_workspace_source_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "saya"
            binary.write_bytes(b"ELF crates/saya-cli/src/agent/tools/workspace/edit/append.rs")
            result = self.check("binary", binary)
            self.assertEqual(result.returncode, 0, result.stderr)
    def test_noncanonical_traversal_and_symlink_entries_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "saya-0.4.2-x86_64-unknown-linux-gnu.tar.gz"
            tar_gz(
                archive,
                {
                    "saya-0.4.2-x86_64-unknown-linux-gnu/../.env": b"private",
                },
            )
            self.assert_rejected("archive", archive)
            link = root / "saya-0.4.2-linux.zip"
            link_root = "saya-0.4.2-linux"
            with zipfile.ZipFile(link, "w") as archive:
                item = zipfile.ZipInfo(f"{link_root}/link")
                item.create_system = 3
                item.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(item, "saya")
            self.assert_rejected("archive", link)
            symlink = root / "saya-0.4.2-x86_64-unknown-linux-gnu.tar.gz"
            tar_gz(
                symlink,
                {"saya-0.4.2-x86_64-unknown-linux-gnu/saya": b"binary"},
                "saya-0.4.2-x86_64-unknown-linux-gnu/link",
            )
            self.assert_rejected("archive", symlink)
    def test_malformed_and_encrypted_archives_fail_with_clean_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            malformed = root / "saya-0.4.2-linux.tar.gz"
            malformed.write_bytes(b"not a tar or zip archive")
            result = self.check("archive", malformed)
            self.assert_rejected("archive", malformed)
            self.assertIn("artifact content check failed:", result.stderr)
            encrypted = root / "saya-0.4.2-linux.zip"
            zip_file(encrypted, {"saya-0.4.2-linux/saya": b"binary"})
            payload = bytearray(encrypted.read_bytes())
            local_header = payload.index(b"PK\x03\x04")
            central_header = payload.index(b"PK\x01\x02")
            payload[local_header + 6] |= 1
            payload[central_header + 8] |= 1
            encrypted.write_bytes(payload)
            result = self.check("archive", encrypted)
            self.assert_rejected("archive", encrypted)
            self.assertIn("encrypted ZIP entry", result.stderr)
