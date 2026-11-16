"""file_limits: the physical-line file length checker for saya."""

import sys
from pathlib import Path

from .counting import count_physical_lines
from .policy import PolicyError, load_policy
from .report import render_json, render_text, violation_entry
from .views import enumerate_view, is_valid_view

DEFAULT_CONFIG = Path(".config") / "file-limits.json"


def _emit(stream, text: str):
    buffer = getattr(stream, "buffer", None)
    if buffer is None:
        stream.write(text)
    else:
        buffer.write(text.encode("utf-8", "surrogateescape"))
    stream.flush()


def _valid_view(view: str) -> bool:
    return is_valid_view(view)


def run_strict(*, repo, view, config, as_json, stdout, stderr) -> int:
    """Run strict mode; return the process exit code (0, 1 or 3)."""
    if not _valid_view(view):
        _emit(stderr, f"error: unknown view {view!r}\n")
        return 3
    config_path = Path(config) if config is not None else Path(repo) / DEFAULT_CONFIG
    try:
        policy = load_policy(config_path)
    except PolicyError as exc:
        _emit(stderr, f"error: {exc}\n")
        return 3
    sources, errors = enumerate_view(repo, view, policy)
    violations = []
    review_candidates = []
    for path, data in sources:
        lines = count_physical_lines(data)
        if lines >= policy.review:
            review_candidates.append({"path": path, "lines": lines})
        if lines > policy.limit:
            violations.append(violation_entry(path, lines, policy.limit))
    violations.sort(key=lambda entry: (-entry["lines"], entry["path"]))
    review_candidates.sort(key=lambda entry: (-entry["lines"], entry["path"]))
    if as_json:
        _emit(
            stdout,
            render_json(
                mode="strict",
                view=view,
                limit=policy.limit,
                target=policy.target,
                review=policy.review,
                files_checked=len(sources),
                violations=violations,
                review_candidates=review_candidates,
                errors=errors,
            ),
        )
    else:
        text, err_text = render_text(
            limit=policy.limit,
            target=policy.target,
            review=policy.review,
            files_checked=len(sources),
            violations=violations,
            review_candidates=review_candidates,
            errors=errors,
        )
        _emit(stdout, text)
        _emit(stderr, err_text)
    if errors:
        return 3
    return 1 if violations else 0
