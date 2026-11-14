from .test_support import *


class ArchiveStructureCases:
    def test_missing_tar_header_hook_is_rejected_before_parser(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            crate = Path(tmp) / "saya-types-0.4.2.crate"
            valid_crate(crate)
            for missing_mode in ("noncallable", "absent"):
                with self.subTest(missing_mode=missing_mode):
                    with mock.patch.object(tarfile, "open", side_effect=AssertionError("TAR parser entered")):
                        with mock.patch.object(tarfile.TarInfo, "_fromtarfile", new=None):
                            if missing_mode == "absent":
                                delattr(tarfile.TarInfo, "_fromtarfile")
                            with self.assertRaisesRegex(GATE_MODULE.Rejected, "required tarfile hook"):
                                GATE_MODULE.scan_archive(str(crate), "crate")
    def test_intended_release_archive_and_crate_are_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "saya-0.4.2-x86_64-unknown-linux-gnu.tar.gz"
            tar_gz(
                archive,
                {
                    "saya-0.4.2-x86_64-unknown-linux-gnu/saya": b"ELF public binary",
                    "saya-0.4.2-x86_64-unknown-linux-gnu/README.md": b"public docs",
                    "saya-0.4.2-x86_64-unknown-linux-gnu/LICENSE": b"license",
                    "saya-0.4.2-x86_64-unknown-linux-gnu/SECURITY.md": b"security policy",
                },
            )
            crate = root / "saya-types-0.4.2.crate"
            tar_gz(
                crate,
                {
                    "saya-types-0.4.2/Cargo.toml": b'[package]\nlicense = "Apache-2.0"',
                    "saya-types-0.4.2/Cargo.toml.orig": b"[package]",
                    "saya-types-0.4.2/Cargo.lock": b"version = 4",
                    "saya-types-0.4.2/src/lib.rs": b"pub struct PublicType;",
                    "saya-types-0.4.2/LICENSE": b"license",
                },
            )
            self.assertEqual(self.check("archive", archive).returncode, 0)
            windows = root / "saya-0.4.2-x86_64-pc-windows-msvc.zip"
            zip_file(
                windows,
                {
                    "saya-0.4.2-x86_64-pc-windows-msvc/saya.exe": b"MZ public binary",
                    "saya-0.4.2-x86_64-pc-windows-msvc/README.md": b"public docs",
                    "saya-0.4.2-x86_64-pc-windows-msvc/LICENSE": b"license",
                    "saya-0.4.2-x86_64-pc-windows-msvc/SECURITY.md": b"security policy",
                },
            )
            self.assertEqual(self.check("archive", windows).returncode, 0)
            self.assertEqual(self.check("crate", crate).returncode, 0)
    def test_sparse_tar_forms_are_rejected_before_stdlib_sparse_parsers(self) -> None:
        cases = (
            ("old GNU", None, "_proc_sparse"),
            ("PAX 0.0", "_proc_gnusparse_00", "_proc_gnusparse_00"),
            ("PAX 0.1", "_proc_gnusparse_01", "_proc_gnusparse_01"),
            ("PAX 1.0", "_proc_gnusparse_10", "_proc_gnusparse_10"),
        )
        with tempfile.TemporaryDirectory() as tmp:
            for label, pax_parser, parser in cases:
                with self.subTest(label=label):
                    crate = Path(tmp) / f"saya-types-0.4.2-{label.replace(' ', '-')}.crate"
                    sparse_crate(crate, pax_parser=pax_parser)
                    with mock.patch.object(tarfile.TarInfo, parser, side_effect=AssertionError("sparse parser entered")):
                        with self.assertRaisesRegex(GATE_MODULE.Rejected, "sparse"):
                            GATE_MODULE.scan_archive(str(crate), "crate")
    def test_both_binary_names_are_rejected_even_when_only_alternate_leaks(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "saya-0.4.2-x86_64-pc-windows-msvc.zip"
            prefix = "saya-0.4.2-x86_64-pc-windows-msvc"
            zip_file(
                archive,
                {
                    f"{prefix}/saya.exe": b"MZ clean selected binary",
                    f"{prefix}/saya": b"ELF /Users/builduser/private-worktree/src/main.rs",
                    f"{prefix}/README.md": b"docs",
                    f"{prefix}/LICENSE": b"license",
                    f"{prefix}/SECURITY.md": b"policy",
                },
            )
            result = self.check("archive", archive)
            self.assert_rejected("archive", archive)
            self.assertIn("exactly one executable", result.stderr)
    def test_windows_zip_with_explicit_root_directory_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "saya-0.4.2-x86_64-pc-windows-msvc.zip"
            prefix = "saya-0.4.2-x86_64-pc-windows-msvc"
            zip_file(
                archive,
                {
                    f"{prefix}/": None,
                    f"{prefix}/saya.exe": b"MZ public binary",
                    f"{prefix}/README.md": b"public docs",
                    f"{prefix}/LICENSE": b"license",
                    f"{prefix}/SECURITY.md": b"security policy",
                },
            )
            self.assertEqual(self.check("archive", archive).returncode, 0)
    def test_credential_filenames_are_rejected_only_for_non_code_payloads(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            safe = root / "saya-agent-0.4.2.crate"
            valid_crate(safe, {"src/session.rs": b"pub struct Session;", "src/secret.rs": b"pub struct Secret;"})
            self.assertEqual(self.check("crate", safe).returncode, 0)
            for filename in ("credentials.toml", "credentials.json"):
                crate = root / f"saya-agent-0.4.2-{filename.split('.')[1]}.crate"
                valid_crate(crate, {f"src/{filename}": b"synthetic credential"})
                result = self.check("crate", crate)
                self.assert_rejected("crate", crate)
                self.assertIn("credential", result.stderr.lower())

    def test_session_test_source_is_allowed_but_private_session_paths_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crate = root / "saya-store-0.4.2.crate"
            valid_crate(
                crate,
                {
                    "tests/sessions/atomicity.rs": b"#[test]\nfn atomicity() {}\n",
                },
            )
            result = self.check("crate", crate)
            self.assertEqual(result.returncode, 0, result.stderr)

            for index, payload in enumerate(
                (
                    "sessions/session.json",
                    "tests/sessions/session.json",
                    "tests/sessions/credentials.rs",
                    "tests/sessions/secrets.rs",
                    "tests/sessions/credentials.toml",
                    "tests/sessions/.env.rs",
                    "tests/sessions/.git/leaked.rs",
                    "tests/sessions/helpers/leaked.rs",
                )
            ):
                with self.subTest(payload=payload):
                    bad = root / f"saya-store-0.4.2-private-{index}.crate"
                    valid_crate(bad, {payload: b"private runtime data"})
                    self.assert_rejected("crate", bad)

    def test_crate_requires_regular_source_and_rejects_colon_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            empty = root / "saya-agent-0.4.2.crate"
            with tarfile.open(empty, "w:gz") as archive:
                for name, content in {
                    "saya-agent-0.4.2/Cargo.toml": b'[package]\nlicense = "Apache-2.0"',
                    "saya-agent-0.4.2/src/": None,
                    "saya-agent-0.4.2/src/empty/": None,
                }.items():
                    info = tarfile.TarInfo(name)
                    if content is None:
                        info.type = tarfile.DIRTYPE
                        archive.addfile(info)
                    else:
                        info.size = len(content)
                        archive.addfile(info, io.BytesIO(content))
            result = self.check("crate", empty)
            self.assert_rejected("crate", empty)
            self.assertIn("regular source", result.stderr)
            colon = root / "saya-agent-0.4.3.crate"
            valid_crate(colon, {"src/config.rs:secret": b"not an NTFS stream"})
            self.assert_rejected("crate", colon)
    def test_archive_budget_is_checked_before_opening_the_over_budget_entry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya-0.4.2-test.zip"
            zip_file(path, {"root/first": b"1234", "root/second": b"5678"})
            gate = GATE_MODULE
            original_open = zipfile.ZipFile.open

            def guarded_open(archive, name, *args, **kwargs):
                filename = name.filename if isinstance(name, zipfile.ZipInfo) else name
                if filename == "root/second":
                    raise AssertionError("over-budget payload was opened")
                return original_open(archive, name, *args, **kwargs)

            with mock.patch.object(gate, "MAX_ARCHIVE_BYTES", 4):
                with mock.patch.object(zipfile.ZipFile, "open", guarded_open):
                    with self.assertRaisesRegex(gate.Rejected, "size limit"):
                        gate.scan_archive(str(path), "archive")
    def test_archive_input_limit_rejects_before_opening_the_container(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya-0.4.2-linux.zip"
            zip_file(path, {"saya-0.4.2-linux/saya": b"binary"})
            with mock.patch.object(GATE_MODULE, "MAX_ARCHIVE_INPUT_BYTES", 1, create=True):
                with mock.patch.object(GATE_MODULE, "archive_entries", side_effect=AssertionError("container opened")):
                    with self.assertRaisesRegex(GATE_MODULE.Rejected, "input size limit"):
                        GATE_MODULE.scan_archive(str(path), "archive")
    def test_zip_metadata_limit_rejects_before_stdlib_central_directory_allocation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "saya-0.4.2-linux.zip"
            prefix = "saya-0.4.2-linux"
            zip_file(path, {
                f"{prefix}/saya": b"binary",
                f"{prefix}/README.md": b"docs",
                f"{prefix}/LICENSE": b"license",
                f"{prefix}/SECURITY.md": b"policy",
            })
            with mock.patch.object(GATE_MODULE, "MAX_ZIP_METADATA_BYTES", 8, create=True):
                with mock.patch.object(zipfile.ZipFile, "__init__", side_effect=AssertionError("ZIP central directory parsed")):
                    with self.assertRaisesRegex(GATE_MODULE.Rejected, "ZIP metadata size limit"):
                        GATE_MODULE.scan_archive(str(path), "archive")
            with mock.patch.object(GATE_MODULE, "MAX_ARCHIVE_ENTRIES", 0):
                with mock.patch.object(zipfile.ZipFile, "__init__", side_effect=AssertionError("ZIP entries allocated")):
                    with self.assertRaisesRegex(GATE_MODULE.Rejected, "too many entries"):
                        GATE_MODULE.scan_archive(str(path), "archive")
