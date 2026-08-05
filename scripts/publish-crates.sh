#!/usr/bin/env bash
#
# Publish the SAYA workspace crates to crates.io in dependency order.
#
# Prerequisites:
#   * A crates.io API token with publish scope. Either run `cargo login` once,
#     or export CARGO_REGISTRY_TOKEN=<token> before running this script.
#   * A clean checkout of the tag you intend to publish (versions must match).
#   * `cargo package -p saya-cli` succeeds locally.
#
# crates.io publishing is PERMANENT (versions can be yanked but never deleted
# or overwritten), so review the versions before running. Publish order follows
# the dependency DAG: a crate can only be published after every crate it depends
# on is already indexed.
#
# Usage:
#   ./scripts/publish-crates.sh            # publish for real
#   DRY_RUN=1 ./scripts/publish-crates.sh  # print what would happen, publish nothing
set -euo pipefail

CRATES=(saya-types saya-config saya-store saya-agent saya-connectors saya-cli)
DRY_RUN="${DRY_RUN:-0}"

cd "$(dirname "$0")/.."

for crate in "${CRATES[@]}"; do
  echo "==> $crate"
  if [ "$DRY_RUN" = "1" ]; then
    cargo publish -p "$crate" --dry-run --allow-dirty
  else
    # cargo (>= 1.66) waits for the crate to be indexed before returning, so a
    # dependent published next can resolve it. The sleep is a belt-and-braces
    # fallback for index propagation lag.
    cargo publish -p "$crate"
    [ "$crate" != "saya-cli" ] && sleep 20
  fi
done

echo "Done. If publishing for real, verify: https://crates.io/crates/saya-cli"
