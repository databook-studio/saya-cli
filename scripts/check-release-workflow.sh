#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release-candidate.yml"
CI_WORKFLOW="$ROOT_DIR/.github/workflows/ci.yml"

ruby -ryaml "$ROOT_DIR/scripts/check_release_workflow/release.rb" "$WORKFLOW"
ruby -ryaml "$ROOT_DIR/scripts/check_release_workflow/ci.rb" "$CI_WORKFLOW" "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" "$ROOT_DIR"
ruby -ryaml -ropen3 -rtmpdir "$ROOT_DIR/scripts/check_release_workflow/tap.rb" "$WORKFLOW" "$ROOT_DIR"
ruby -rfileutils -rjson -ropen3 -rtmpdir "$ROOT_DIR/scripts/check_release_workflow/publish.rb" "$ROOT_DIR"
