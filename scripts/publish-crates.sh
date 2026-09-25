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

CRATES=(saya-types saya-config saya-store saya-agent saya-connectors saya-harness saya-cli)
DRY_RUN="${DRY_RUN:-0}"
UA="saya-release (github.com/databook-studio/saya-cli)"
METADATA=""

crate_index() {
  local i
  for i in "${!CRATES[@]}"; do
    if [ "${CRATES[$i]}" = "$1" ]; then
      printf '%s\n' "$i"
      return 0
    fi
  done
  printf '%s\n' '-1'
}

validate_workspace() {
  METADATA="$(cargo metadata --locked --no-deps --format-version=1)"
  local release_version workspace_names expected_names actual_names
  release_version="$(jq -er '[.packages[] | select(.name == "saya-cli") | .version] | if length == 1 then .[0] else error("saya-cli package missing or duplicated") end' <<<"$METADATA")"
  workspace_names="$(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) | .name] | .[]' <<<"$METADATA")"
  expected_names="$(printf '%s\n' "${CRATES[@]}" | LC_ALL=C sort)"
  actual_names="$(printf '%s\n' "$workspace_names" | LC_ALL=C sort)"
  if [ "$actual_names" != "$expected_names" ]; then
    echo "publish order does not cover the workspace exactly" >&2
    echo "expected: ${CRATES[*]}" >&2
    echo "workspace: $(tr '\n' ' ' <<<"$workspace_names")" >&2
    exit 1
  fi

  while IFS=$'\t' read -r crate version; do
    if [ "$version" != "$release_version" ]; then
      echo "workspace crate $crate has version $version; expected $release_version" >&2
      exit 1
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) | [.name, .version] | @tsv] | .[]' <<<"$METADATA")

  while IFS=$'\t' read -r crate dependency requirement; do
    if [ "$requirement" != "^$release_version" ]; then
      echo "workspace dependency $crate -> $dependency requires $requirement; expected ^$release_version" >&2
      exit 1
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) as $package | $package.dependencies[]? | select(.path != null) | [$package.name, .name, .req] | @tsv] | .[]' <<<"$METADATA")

  while IFS=$'\t' read -r crate dependency; do
    local crate_position dependency_position
    crate_position="$(crate_index "$crate")"
    dependency_position="$(crate_index "$dependency")"
    if [ "$dependency_position" -lt 0 ] || [ "$dependency_position" -ge "$crate_position" ]; then
      echo "publish order violates workspace dependency: $dependency must precede $crate" >&2
      exit 1
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) as $package | $package.dependencies[]? | select(.path != null) | [$package.name, .name] | @tsv] | .[]' <<<"$METADATA")
}

validate_workspace

# In CI, skip gracefully if the registry token hasn't been configured yet, so a
# release run doesn't fail before the maintainer opts in.
if [ "${GITHUB_ACTIONS:-}" = "true" ] && [ "$DRY_RUN" != "1" ] && [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
  echo "CARGO_REGISTRY_TOKEN is not set — skipping crates.io publish."
  echo "Add it under Settings > Secrets and variables > Actions to enable this step."
  exit 0
fi

crate_version() {
  jq -er --arg n "$1" '[.packages[] | select(.name == $n) | .version] | if length == 1 then .[0] else error("crate missing or duplicated: \($n)") end' <<<"$METADATA"
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
