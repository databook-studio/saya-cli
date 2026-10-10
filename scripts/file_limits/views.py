"""Source enumeration for the file-limits checker.

Enumerates eligible source paths for a view and yields their bytes:

* ``index``     — tracked files as recorded in the git index (staged bytes,
  never the working copy).
* ``worktree``  — tracked plus non-ignored untracked files, read from disk.
* ``rev:<rev>`` — the tracked tree of that revision.

Any git failure, unreadable file or blob, or symlinked eligible source is
collected as an error; files are never silently skipped.
"""

import os
import subprocess
from pathlib import Path, PurePosixPath

GIT_GLOBAL = ("-c", "core.quotepath=false")
SYMLINK_MODE = b"120000"


class ViewError(Exception):
    """Raised for git-level failures during enumeration."""


def _decode(raw: bytes) -> str:
    return raw.decode("utf-8", "surrogateescape")


def _git(repo, args) -> bytes:
    proc = subprocess.run(
        ["git", *GIT_GLOBAL, "-C", str(repo), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", "replace").strip()
        raise ViewError(f"git {' '.join(args)} failed: {detail}")
    return proc.stdout


def is_valid_view(view: str) -> bool:
    """True for ``index``, ``worktree`` or ``rev:<revision>``."""
    if view in ("index", "worktree"):
        return True
    return view.startswith("rev:") and len(view) > 4


def is_eligible(path: str, policy) -> bool:
    """True when the path is source, outside generated and dependency dirs."""
    parts = PurePosixPath(path).parts
    extensions = {extension.casefold() for extension in policy.extensions}
    excluded = set(policy.exclude_dir_names)
    if PurePosixPath(path).suffix.casefold() not in extensions:
        return False
    source_roots = {"src", "tests", "benches", "examples", "scripts"}
    source_owned = bool(parts and parts[0] in source_roots) or (
        len(parts) > 2 and parts[0] == "crates" and parts[2] in source_roots
    )
    for part in parts:
        if part in excluded and not (
            source_owned and part in {"target", "node_modules"}
        ):
            return False
    return True


def _index_entries(repo):
    raw = _git(repo, ["ls-files", "-z", "-s"])
    entries = {}
    for chunk in raw.split(b"\0"):
        if not chunk:
            continue
        meta, _, raw_path = chunk.partition(b"\t")
        mode, object_id, _stage = meta.split()
        path = _decode(raw_path)
        entries.setdefault(path, (mode, object_id))
    return entries


class _BlobReader:
    """Reads blob contents through one long-lived ``git cat-file --batch``."""

    def __init__(self, repo):
        self._proc = subprocess.Popen(
            ["git", *GIT_GLOBAL, "-C", str(repo), "cat-file", "--batch"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
        )

    def read_blob(self, object_id: bytes, path: str) -> bytes:
        try:
            self._proc.stdin.write(object_id + b"\n")
            self._proc.stdin.flush()
        except OSError as exc:
            raise ViewError(f"cannot read blob for {path}: {exc}") from None
        header = self._proc.stdout.readline()
        fields = header.split()
        if len(fields) != 3 or fields[1] != b"blob":
            raise ViewError(
                f"cannot read blob for {path}: git cat-file returned {header!r}"
            )
        try:
            size = int(fields[2])
        except ValueError:
            raise ViewError(
                f"cannot read blob for {path}: bad size in {header!r}"
            ) from None
        data = self._proc.stdout.read(size)
        terminator = self._proc.stdout.read(1)
        if len(data) != size or terminator != b"\n":
            raise ViewError(f"cannot read blob for {path}: truncated response")
        return data

    def close(self):
        proc, self._proc = self._proc, None
        if proc is None:
            return
        try:
            proc.stdin.close()
            proc.stdout.close()
            proc.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            proc.kill()
            proc.wait()


def _blob_sources(entries, reader):
    sources, errors = [], []
    for path, (mode, object_id) in sorted(entries.items()):
        if mode == SYMLINK_MODE:
            errors.append(f"symlinked source is not allowed: {path}")
            continue
        try:
            sources.append((path, reader.read_blob(object_id, path)))
        except ViewError as exc:
            errors.append(str(exc))
    return sources, errors


def _index_view(repo, policy):
    entries = {
        path: entry
        for path, entry in _index_entries(repo).items()
        if is_eligible(path, policy)
    }
    reader = _BlobReader(repo)
    try:
        return _blob_sources(entries, reader)
    finally:
        reader.close()


def _rev_view(repo, revision, policy):
    raw = _git(repo, ["ls-tree", "-r", "-z", revision])
    entries = {}
    for chunk in raw.split(b"\0"):
        if not chunk:
            continue
        meta, _, raw_path = chunk.partition(b"\t")
        mode, entry_type, object_id = meta.split()
        if entry_type == b"blob":
            entries[_decode(raw_path)] = (mode, object_id)
    entries = {
        path: entry for path, entry in entries.items() if is_eligible(path, policy)
    }
    reader = _BlobReader(repo)
    try:
        return _blob_sources(entries, reader)
    finally:
        reader.close()


def _worktree_view(repo, policy):
    tracked = {
        path: entry
        for path, entry in _index_entries(repo).items()
        if is_eligible(path, policy)
    }
    raw = _git(repo, ["ls-files", "-z", "--others", "--exclude-standard"])
    untracked = {
        _decode(chunk) for chunk in raw.split(b"\0") if chunk
    }
    untracked = {path for path in untracked if is_eligible(path, policy)}
    sources, errors = [], []
    for path in sorted(tracked.keys() | untracked):
        mode = tracked[path][0] if path in tracked else None
        if mode == SYMLINK_MODE or os.path.islink(Path(repo) / path):
            errors.append(f"symlinked source is not allowed: {path}")
            continue
        try:
            sources.append((path, (Path(repo) / path).read_bytes()))
        except OSError as exc:
            errors.append(f"cannot read file {path}: {exc}")
    return sources, errors


def enumerate_view(repo, view: str, policy):
    """Return ``(sources, errors)`` for the view; never raises."""
    try:
        if view == "index":
            return _index_view(repo, policy)
        if view == "worktree":
            return _worktree_view(repo, policy)
        if view.startswith("rev:") and len(view) > 4:
            return _rev_view(repo, view[4:], policy)
        raise ViewError(
            f"unknown view {view!r}: expected index, worktree or rev:<revision>"
        )
    except ViewError as exc:
        return [], [str(exc)]
