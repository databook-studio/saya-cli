#!/usr/bin/env bash
#
# Publish the SAYA workspace crates to crates.io in dependency order.
#
# Idempotent: any crate whose exact version is already on crates.io is skipped,
# so this is safe to re-run after a partial publish (e.g. a rate-limit stop) and
# safe to run unconditionally from CI on every release tag.
#
# Auth:
#   * Locally: run `cargo login` once (token stored in ~/.cargo/credentials).
#   * In CI: set the CARGO_REGISTRY_TOKEN secret; cargo reads it from the env.
#
# crates.io publishing is PERMANENT (versions can be yanked but never deleted or
# overwritten). Publish order follows the dependency DAG.
#
# Usage:
#   ./scripts/publish-crates.sh            # publish (skips already-published)
#   DRY_RUN=1 ./scripts/publish-crates.sh  # verify only, publish nothing
set -euo pipefail
cd "$(dirname "$0")/.."

CRATES=(saya-types saya-config saya-store saya-agent saya-connectors saya-cli)
DRY_RUN="${DRY_RUN:-0}"
UA="saya-release (github.com/databook-studio/saya-cli)"

# In CI, skip gracefully if the registry token hasn't been configured yet, so a
# release run doesn't fail before the maintainer opts in.
if [ "${GITHUB_ACTIONS:-}" = "true" ] && [ "$DRY_RUN" != "1" ] && [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
  echo "CARGO_REGISTRY_TOKEN is not set — skipping crates.io publish."
  echo "Add it under Settings > Secrets and variables > Actions to enable this step."
  exit 0
fi

crate_version() {
  cargo metadata --no-deps --format-version=1 | jq -r --arg n "$1" '.packages[] | select(.name==$n) | .version'
}

is_published() {
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' -H "User-Agent: $UA" "https://crates.io/api/v1/crates/$1/$2")
  [ "$code" = "200" ]
}

for crate in "${CRATES[@]}"; do
  version=$(crate_version "$crate")
  if is_published "$crate" "$version"; then
    echo "== skip $crate $version (already on crates.io)"
    continue
  fi
  echo "== publish $crate $version"
  if [ "$DRY_RUN" = "1" ]; then
    cargo publish -p "$crate" --dry-run --allow-dirty
  else
    cargo publish -p "$crate"
    # cargo (>= 1.66) waits for the crate to be indexed; this is a small extra
    # margin so a dependent published next can resolve it.
    [ "$crate" != "saya-cli" ] && sleep 15
  fi
done

echo "Done."
