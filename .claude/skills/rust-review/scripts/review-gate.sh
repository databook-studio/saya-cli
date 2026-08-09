#!/usr/bin/env bash
# saya local review gate — the checks a change must pass before it is committed.
# Mirrors the CI `verify` job (see docs/standards/workflow.md). Run from the repo root.
#
#   .claude/skills/rust-review/scripts/review-gate.sh
#
# Uses cargo-nextest when available, falling back to `cargo test`. Runs `cargo audit`
# only when it is installed (CI runs it on dependency changes).
set -euo pipefail

run() { printf '\n\033[1m▶ %s\033[0m\n' "$*"; "$@"; }

run cargo fmt --check
run cargo clippy --workspace --all-targets --locked -- -D warnings

if command -v cargo-nextest >/dev/null 2>&1; then
  run cargo nextest run --workspace --locked
else
  echo "note: cargo-nextest not installed; falling back to cargo test" >&2
  run cargo test --workspace --locked
fi

# nextest does not run doctests; cover them explicitly.
run cargo test --workspace --doc --locked

if command -v cargo-audit >/dev/null 2>&1; then
  run cargo audit --deny warnings
else
  echo "note: cargo-audit not installed; skipping (CI enforces it)" >&2
fi

printf '\n\033[32m✓ review gate passed\033[0m\n'
