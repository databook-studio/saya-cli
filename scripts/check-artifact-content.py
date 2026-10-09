#!/usr/bin/env python3
"""Reject unexpected files and local build data in Saya release artifacts."""

from pathlib import Path

# Keep helper declarations in this module namespace: the existing importlib
# tests patch its limits and parser functions at this executable entry path.
_HELPERS = Path(__file__).resolve().parent / "artifact_content"
for _name in ("common.py", "traversal.py", "scanner.py"):
    _path = _HELPERS / _name
    exec(compile(_path.read_bytes(), str(_path), "exec"))

if __name__ == "__main__":
    raise SystemExit(main())
