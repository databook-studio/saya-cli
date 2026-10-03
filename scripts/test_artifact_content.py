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

ROOT = Path(__file__).resolve().parents[1]
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


class ArtifactContentTests(unittest.TestCase):
    def check(self, kind: str, path: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(GATE), kind, str(path)],
            text=True,
            capture_output=True,
            check=False,
        )

    def assert_rejected(self, kind: str, path: Path) -> None:
        result = self.check(kind, path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact content check failed:", result.stderr)

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


class ReleaseScriptIntegrationTests(unittest.TestCase):
    def executable(self, path: Path, content: str) -> None:
        path.write_text(content)
        path.chmod(0o755)

    def test_release_build_env_preserves_flags_and_paths_with_spaces(self) -> None:
        helper = ROOT / "scripts" / "release-build-env.sh"
        command = (
            f'source "{helper}"; configure_saya_release_build_env "$WORKSPACE"; '
            'printf "%s\\0%s\\0%s" "$CARGO_ENCODED_RUSTFLAGS" "$CXXFLAGS" "$CC_SHELL_ESCAPED_FLAGS"'
        )
        env = os.environ | {
            "WORKSPACE": "/tmp/saya workspace",
            "CARGO_HOME": "/tmp/cargo home",
            "CARGO_ENCODED_RUSTFLAGS": "-C\x1fopt-level=3",
            "RUSTFLAGS": "-C opt-level=1",
            "CXXFLAGS": "-DSAYA_EXISTING=1",
        }
        result = subprocess.run(["bash", "-c", command], cwd=ROOT, env=env, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        encoded, cxxflags, escaped = result.stdout.decode().split("\0")
        self.assertEqual(encoded.split("\x1f"), [
            "-C", "opt-level=3",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])
        parsed_cxx = shlex.split(cxxflags)
        self.assertEqual(parsed_cxx, [
            "-DSAYA_EXISTING=1",
            "-ffile-prefix-map=/tmp/saya workspace=/saya",
            "-ffile-prefix-map=/tmp/cargo home=/cargo",
        ])
        self.assertEqual(escaped, "1")
        legacy_env = dict(env)
        legacy_env.pop("CARGO_ENCODED_RUSTFLAGS")
        legacy_env.pop("RUSTFLAGS")
        legacy_env["RUSTFLAGS"] = "-C debuginfo=0"
        legacy = subprocess.run(["bash", "-c", command], cwd=ROOT, env=legacy_env, capture_output=True, check=False)
        self.assertEqual(legacy.returncode, 0, legacy.stderr.decode())
        legacy_encoded = legacy.stdout.decode().split("\0")[0].split("\x1f")
        self.assertEqual(legacy_encoded, [
            "-C", "debuginfo=0",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

    def test_release_build_env_preserves_multiline_and_encoded_argument_flags(self) -> None:
        helper = ROOT / "scripts" / "release-build-env.sh"
        command = (
            f'source "{helper}"; configure_saya_release_build_env "$WORKSPACE"; '
            'printf "%s" "$CARGO_ENCODED_RUSTFLAGS"'
        )
        base = os.environ | {
            "WORKSPACE": "/tmp/saya workspace",
            "CARGO_HOME": "/tmp/cargo home",
        }
        legacy_env = dict(base)
        legacy_env.pop("CARGO_ENCODED_RUSTFLAGS", None)
        legacy_env["RUSTFLAGS"] = "-C opt-level=3\n-C debuginfo=0"
        legacy = subprocess.run(["bash", "-c", command], cwd=ROOT, env=legacy_env, capture_output=True, check=False)
        self.assertEqual(legacy.returncode, 0, legacy.stderr.decode())
        self.assertEqual(legacy.stdout.decode().split("\x1f"), [
            "-C", "opt-level=3", "-C", "debuginfo=0",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

        encoded_env = dict(base)
        encoded_env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f--cfg\x1ffeature=custom build\x1f"
        encoded_env["RUSTFLAGS"] = "-C opt-level=3\n-C debuginfo=0"
        encoded = subprocess.run(["bash", "-c", command], cwd=ROOT, env=encoded_env, capture_output=True, check=False)
        self.assertEqual(encoded.returncode, 0, encoded.stderr.decode())
        self.assertEqual(encoded.stdout.decode().split("\x1f"), [
            "", "--cfg", "feature=custom build", "",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

        empty_encoded_env = dict(encoded_env)
        empty_encoded_env["CARGO_ENCODED_RUSTFLAGS"] = ""
        empty_encoded = subprocess.run(["bash", "-c", command], cwd=ROOT, env=empty_encoded_env, capture_output=True, check=False)
        self.assertEqual(empty_encoded.returncode, 0, empty_encoded.stderr.decode())
        self.assertEqual(empty_encoded.stdout.decode().split("\x1f"), [
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

    def test_package_script_uses_custom_target_dir_with_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "stub bin"
            bin_dir.mkdir()
            target = root / "custom target dir"
            output = root / "package output"
            stub_binary = root / "stub saya"
            stub_binary.write_text(
                "#!/bin/sh\n"
                'if [ "$1" = "--version" ]; then echo STUB_BUILD_OUTPUT; exit 0; fi\n'
                'if [ "$1" = "--non-interactive" ]; then exit 3; fi\n'
                "exit 0\n"
            )
            stub_binary.chmod(0o755)
            self.executable(bin_dir / "rustc", "#!/bin/sh\necho 'host: x86_64-unknown-linux-gnu'\n")
            self.executable(
                bin_dir / "cargo",
                "#!/bin/sh\nset -eu\n"
                'test "$1" = build\n'
                'mkdir -p "$CARGO_TARGET_DIR/release"\n'
                'cp "$SAYA_STUB_BINARY" "$CARGO_TARGET_DIR/release/saya"\n'
                'chmod +x "$CARGO_TARGET_DIR/release/saya"\n',
            )
            env = os.environ | {
                "PATH": f"{bin_dir}:{os.environ['PATH']}",
                "CARGO_TARGET_DIR": str(target),
                "SAYA_PACKAGE_DIR": str(output),
                "SAYA_STUB_BINARY": str(stub_binary),
            }
            result = subprocess.run(["bash", "scripts/package.sh"], cwd=ROOT, env=env, text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((target / "release" / "saya").exists())
            archive_path = next(output.glob("*.tar.gz"))
            prefix = archive_path.name.removesuffix(".tar.gz")
            with tarfile.open(archive_path, "r:gz") as archive:
                self.assertEqual(archive.extractfile(f"{prefix}/saya").read(), stub_binary.read_bytes())

    def test_publish_script_uses_custom_target_dir_with_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "stub bin"
            fixtures = root / "crate fixtures"
            target = root / "custom target dir"
            bin_dir.mkdir()
            fixtures.mkdir()
            metadata = subprocess.run(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
                cwd=ROOT, text=True, capture_output=True, check=True,
            ).stdout
            packages = ["saya-types", "saya-config", "saya-store", "saya-agent", "saya-connectors", "saya-harness", "saya-cli"]
            for crate in packages:
                fixture = fixtures / f"{crate}-0.4.2.crate"
                valid_crate(fixture, {"src/lib.rs": b"pub fn packaged() {}"})
            self.executable(
                bin_dir / "cargo",
                "#!/bin/sh\nset -eu\n"
                'case "$1" in\n'
                'metadata) cat "$SAYA_METADATA_FIXTURE" ;;\n'
                'package) shift; crate=""; while [ "$#" -gt 0 ]; do if [ "$1" = -p ]; then crate="$2"; shift 2; else shift; fi; done; mkdir -p "$CARGO_TARGET_DIR/package"; cp "$SAYA_CRATE_FIXTURES/$crate-0.4.2.crate" "$CARGO_TARGET_DIR/package/$crate-0.4.2.crate" ;;\n'
                'publish) exit 0 ;;\n'
                '*) exit 2 ;;\n'
                'esac\n',
            )
            self.executable(bin_dir / "curl", "#!/bin/sh\nprintf 404\n")
            metadata_path = root / "metadata.json"
            metadata_path.write_text(metadata)
            env = os.environ | {
                "PATH": f"{bin_dir}:{os.environ['PATH']}",
                "CARGO_TARGET_DIR": str(target),
                "SAYA_METADATA_FIXTURE": str(metadata_path),
                "SAYA_CRATE_FIXTURES": str(fixtures),
                "DRY_RUN": "1",
            }
            result = subprocess.run(["bash", "scripts/publish-crates.sh"], cwd=ROOT, env=env, text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(len(list((target / "package").glob("*.crate"))), len(packages))
            self.assertEqual(result.stdout.count(f"artifact content check passed: crate {target}/package/"), len(packages))


if __name__ == "__main__":
    unittest.main()
