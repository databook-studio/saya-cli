#!/usr/bin/env python3
"""Thin CLI entry point for the file-limits checker."""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from file_limits import run_strict


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Enforce physical line limits on first-party sources."
    )
    parser.add_argument(
        "--view",
        default="worktree",
        help="index, worktree or rev:<revision> (default: worktree)",
    )
    parser.add_argument(
        "--repo", default=".", help="repository root to check (default: .)"
    )
    parser.add_argument(
        "--config",
        default=None,
        help="policy JSON path (default: <repo>/.config/file-limits.json)",
    )
    parser.add_argument(
        "--json", action="store_true", help="emit machine-readable JSON"
    )
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    return run_strict(
        repo=args.repo,
        view=args.view,
        config=args.config,
        as_json=args.json,
        stdout=sys.stdout,
        stderr=sys.stderr,
    )


if __name__ == "__main__":
    sys.exit(main())
