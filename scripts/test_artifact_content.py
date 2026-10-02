#!/usr/bin/env python3
"""Behavior tests for the release artifact content gate."""

import io
import subprocess
import stat
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "scripts" / "check-artifact-content.py"
SENTINEL = b"SAYA_ARTIFACT_FORBIDDEN_SENTINEL"


def tar_gz(path: Path, entries: dict[str, bytes], symlink: str | None = None) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for name, content in entries.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))
        if symlink:
            info = tarfile.TarInfo(symlink)
            info.type = tarfile.SYMTYPE
            info.linkname = "saya"
            archive.addfile(info)


def zip_file(path: Path, entries: dict[str, bytes]) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        for name, content in entries.items():
            archive.writestr(name, content)


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


if __name__ == "__main__":
    unittest.main()
