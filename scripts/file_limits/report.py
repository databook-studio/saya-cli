"""Text and JSON diagnostics for the file-limits checker."""

import json


def violation_entry(path: str, lines: int, limit: int) -> dict:
    return {"path": path, "lines": lines, "limit": limit, "excess": lines - limit}


def render_json(
    *, mode, view, limit, target, review, files_checked, violations,
    review_candidates, errors
) -> str:
    document = {
        "mode": mode,
        "view": view,
        "limit": limit,
        "target": target,
        "review": review,
        "files_checked": files_checked,
        "violations": violations,
        "review_candidates": review_candidates,
        "errors": list(errors),
    }
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def render_text(
    *, limit, target, review, files_checked, violations, review_candidates, errors
) -> tuple[str, str]:
    """Return ``(stdout_text, stderr_text)`` for strict mode."""
    lines = [
        f"{entry['path']}: {entry['lines']} lines, limit {entry['limit']},"
        f" excess {entry['excess']}"
        for entry in violations
    ]
    lines.append(
        f"policy: target {target}, review at {review} lines, hard limit {limit}"
    )
    lines.extend(
        f"review candidate: {entry['path']}: {entry['lines']} lines"
        for entry in review_candidates
    )
    lines.append(f"{files_checked} files checked, {len(violations)} over {limit}")
    stderr = "".join(f"error: {message}\n" for message in errors)
    return "\n".join(lines) + "\n", stderr
