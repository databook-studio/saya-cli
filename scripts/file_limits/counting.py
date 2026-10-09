"""The single physical-line counting function used everywhere.

A physical line is terminated by an LF byte. A non-empty file without a
trailing LF has one more line than it has LF bytes. CRLF counts once per
line (only the LF byte is counted). Blanks, comments, doc comments,
attributes, test modules and data literals all count. An empty file has
zero lines.
"""


def count_physical_lines(data: bytes) -> int:
    """Count physical lines in raw file bytes."""
    if not data:
        return 0
    lines = data.count(b"\n")
    if not data.endswith(b"\n"):
        lines += 1
    return lines