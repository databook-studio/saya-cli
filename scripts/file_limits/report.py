"""Text and JSON diagnostics for the file-limits checker."""

import json


def violation_entry(path: str, lines: int, limit: int) -> dict:
    return {"path": path, "lines": lines, "limit": limit, "excess": lines - limit}


def render_json(*, mode, view, limit, files_checked, violations, errors) -> str:
    document = {
        "mode": mode,
        "view": view,
        "limit": limit,
        "files_checked": files_checked,
        "violations": violations,
        "errors": list(errors),
    }
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def render_text(*, limit, files_checked, violations, errors) -> tuple[str, str]:
    """Return ``(stdout_text, stderr_text)`` for strict mode."""
    lines = [
        f"{entry['path']}: {entry['lines']} lines, limit {entry['limit']},"
        f" excess {entry['excess']}"
        for entry in violations
    ]
    lines.append(f"{files_checked} files checked, {len(violations)} over {limit}")
    stderr = "".join(f"error: {message}\n" for message in errors)
    return "\n".join(lines) + "\n", stderr