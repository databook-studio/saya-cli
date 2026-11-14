"""Shared fixtures: temporary git repositories and checker invocation."""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parents[2]
REPO_ROOT = SCRIPTS_DIR.parent

if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

STANDARD_POLICY = json.loads((REPO_ROOT / ".config" / "file-limits.json").read_text())

BASELINE_RELPATH = ".config/file-limits-baseline.json"


def baseline_bytes(limit, entries):
    """Baseline JSON bytes for (path, lines) pairs, sorted like the real writer."""
    document = {
        "version": 1,
        "limit": limit,
        "entries": [
            {"path": path, "lines": lines, "blob": "0" * 40}
            for path, lines in sorted(entries)
        ],
    }
    return (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def n_lines(count, *, final_newline=True):
    if count <= 0:
        return b""
    body = b"row\n" * count
    return body if final_newline else body[:-1]


class TempRepo:
    def __init__(self):
        self.path = Path(tempfile.mkdtemp(prefix="file-limits-"))
        self.git("init", "-q")
        self.git("config", "user.email", "checker@example.com")
        self.git("config", "user.name", "Checker Tests")
        self.git("config", "commit.gpgsign", "false")
        self.write_policy()

    def git(self, *args):
        proc = subprocess.run(
            ["git", "-c", "core.quotepath=false", "-C", str(self.path), *args],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if proc.returncode != 0:
            detail = proc.stderr.decode("utf-8", "replace").strip()
            raise AssertionError(f"git {' '.join(args)} failed: {detail}")
        return proc.stdout

    def write(self, relpath, data):
        target = self.path / relpath
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    def write_lines(self, relpath, count, *, final_newline=True):
        self.write(relpath, n_lines(count, final_newline=final_newline))

    def write_policy(self, data=None):
        if data is None:
            data = STANDARD_POLICY
        self.write(".config/file-limits.json", json.dumps(data).encode("utf-8"))

    def commit_all(self, message="commit"):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def destroy(self):
        shutil.rmtree(self.path, ignore_errors=True)


def run_checker(repo_path, *args):
    command = [
        sys.executable,
        str(REPO_ROOT / "scripts" / "check_file_limits.py"),
        "--repo",
        str(repo_path),
        *args,
    ]
    return subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="surrogateescape",
    )
