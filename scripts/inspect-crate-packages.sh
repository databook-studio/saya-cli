#!/usr/bin/env bash
#
# Package-only inspection of every workspace crate.
#
# Packages each workspace crate exactly like publish-crates.sh does (same
# workspace-local dependency patch args), scans every produced .crate with
# scripts/check-artifact-content.py, hashes it, and emits a JSON receipt.
# Unlike publish-crates.sh this NEVER publishes, never contacts crates.io,
# and never skips a crate: every workspace crate is packaged and scanned,
# unconditionally.
#
# Env:
#   INSPECT_RECEIPT=<path>  additionally write the receipt JSON to <path>
#   INSPECT_OFFLINE=1       pass --offline to cargo package
#   CARGO=<tool>            cargo override (default: cargo)
#   PYTHON=<tool>           scanner interpreter override (default: python3)
#   CARGO_TARGET_DIR        honored for package output location
#
# Failure policy: a crate-count mismatch, package failure, missing/empty
# payload, or scanner rejection exits nonzero and writes a receipt with
# all_pass:false naming the failing crate (never a receipt claiming all_pass).
set -euo pipefail
cd "$(dirname "$0")/.."

CRATES=(saya-types saya-config saya-store saya-agent saya-connectors saya-harness saya-cli)
CARGO="${CARGO:-cargo}"
PYTHON="${PYTHON:-python3}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
METADATA=""
ROWS="$(mktemp)"
trap 'rm -f "$ROWS"' EXIT
SOURCE="$(git rev-parse HEAD)"
if [ -z "$(git status --porcelain --untracked-files=no)" ]; then
  DIRTY=false
else
  DIRTY=true
fi
CARGO_VERSION="$("$CARGO" --version)"

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

crate_version() {
  jq -er --arg n "$1" '[.packages[] | select(.name == $n) | .version] | if length == 1 then .[0] else error("crate missing or duplicated: \($n)") end' <<<"$METADATA"
}

emit_receipt() {
  printf '%s\n' "$1"
  if [ -n "${INSPECT_RECEIPT:-}" ]; then
    printf '%s\n' "$1" >"$INSPECT_RECEIPT"
  fi
}

fail_crate() { # $1 = crate name ("" for a pre-flight failure), $2 = reason
  echo "inspection failed (crate: ${1:-<pre-flight>}): $2" >&2
  local rows_json='[]'
  if [ -s "$ROWS" ]; then
    rows_json="$(jq -s '.' "$ROWS")"
  fi
  emit_receipt "$(jq -cn \
    --arg source "$SOURCE" --argjson dirty "$DIRTY" --arg cargo_version "$CARGO_VERSION" \
    --argjson crates "$rows_json" --arg crate "$1" --arg reason "$2" \
    '{source: $source, dirty: $dirty, cargo_version: $cargo_version, crates: $crates,
      failure: {crate: (if $crate == "" then null else $crate end), reason: $reason}, all_pass: false}')"
  exit 1
}

# The crate-owned examples are canonical for package tests; keep the
# workspace-root copies used by docs and users byte-identical before shipping.
validate_distributed_config_examples() {
  local name canonical distributed
  while IFS=$'\t' read -r name canonical_name; do
    canonical="crates/saya-config/examples/$canonical_name"
    distributed="examples/$name"
    if [[ ! -f "$canonical" || -L "$canonical" ]]; then
      fail_crate "saya-config" "canonical example must be a regular file: $canonical"
    fi
    if [[ ! -f "$distributed" || -L "$distributed" ]]; then
      fail_crate "saya-config" "distributed example must be a regular file: $distributed"
    fi
    if ! cmp -s "$canonical" "$distributed"; then
      fail_crate "saya-config" "distributed example differs from canonical copy: $distributed"
    fi
  done <<'EOF'
connections.toml	connections.example.toml
connections.docker.toml	connections.docker.example.toml
EOF
}

validate_distributed_config_examples

# Same checks as publish-crates.sh: the crate list must equal the workspace
# members exactly, versions must agree, and the list must respect the
# workspace dependency order.
validate_workspace() {
  METADATA="$("$CARGO" metadata --locked --no-deps --format-version=1)"
  local release_version workspace_names expected_names actual_names
  release_version="$(jq -er '[.packages[] | select(.name == "saya-cli") | .version] | if length == 1 then .[0] else error("saya-cli package missing or duplicated") end' <<<"$METADATA")"
  workspace_names="$(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) | .name] | .[]' <<<"$METADATA")"
  expected_names="$(printf '%s\n' "${CRATES[@]}" | LC_ALL=C sort)"
  actual_names="$(printf '%s\n' "$workspace_names" | LC_ALL=C sort)"
  if [ "$actual_names" != "$expected_names" ]; then
    fail_crate "" "inspection list does not cover the workspace exactly (expected: ${CRATES[*]}; workspace: $(tr '\n' ' ' <<<"$workspace_names"))"
  fi

  while IFS=$'\t' read -r crate version; do
    if [ "$version" != "$release_version" ]; then
      fail_crate "$crate" "workspace crate $crate has version $version; expected $release_version"
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) | [.name, .version] | @tsv] | .[]' <<<"$METADATA")

  while IFS=$'\t' read -r crate dependency requirement; do
    if [ "$requirement" != "^$release_version" ]; then
      fail_crate "$crate" "workspace dependency $crate -> $dependency requires $requirement; expected ^$release_version"
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) as $package | $package.dependencies[]? | select(.path != null) | [$package.name, .name, .req] | @tsv] | .[]' <<<"$METADATA")

  while IFS=$'\t' read -r crate dependency; do
    local crate_position dependency_position
    crate_position="$(crate_index "$crate")"
    dependency_position="$(crate_index "$dependency")"
    if [ "$dependency_position" -lt 0 ] || [ "$dependency_position" -ge "$crate_position" ]; then
      fail_crate "$crate" "inspection order violates workspace dependency: $dependency must precede $crate"
    fi
  done < <(jq -r '[.workspace_members[] as $id | .packages[] | select(.id == $id) as $package | $package.dependencies[]? | select(.path != null) | [$package.name, .name] | @tsv] | .[]' <<<"$METADATA")
}

validate_workspace

for crate in "${CRATES[@]}"; do
  if ! version="$(crate_version "$crate")"; then
    fail_crate "$crate" "crate missing or duplicated in cargo metadata"
  fi
  echo "== inspect package $crate $version"
  PACKAGE_PATCH_ARGS=()
  for dependency in "${CRATES[@]}"; do
    PACKAGE_PATCH_ARGS+=(--config "patch.crates-io.$dependency.path=\"crates/$dependency\"")
  done
  PACKAGE_ARGS=()
  if [ "${INSPECT_OFFLINE:-0}" = "1" ]; then
    PACKAGE_ARGS+=(--offline)
  fi
  if ! "$CARGO" package "${PACKAGE_PATCH_ARGS[@]}" ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} --locked -p "$crate" --no-verify --allow-dirty; then
    fail_crate "$crate" "cargo package failed"
  fi
  PACKAGE="$TARGET_DIR/package/$crate-$version.crate"
  if [ ! -s "$PACKAGE" ]; then
    fail_crate "$crate" "cargo package did not produce a non-empty $PACKAGE"
  fi
  if ! "$PYTHON" scripts/check-artifact-content.py crate "$PACKAGE"; then
    fail_crate "$crate" "artifact content scan rejected $PACKAGE"
  fi
  digest="$(shasum -a 256 "$PACKAGE" | awk '{print $1}')"
  bytes="$(wc -c <"$PACKAGE" | tr -d '[:space:]')"
  jq -cn --arg name "$crate" --arg version "$version" --arg package_path "$PACKAGE" \
    --argjson bytes "$bytes" --arg sha256 "$digest" --arg scan pass \
    '{name: $name, version: $version, package_path: $package_path, bytes: $bytes, sha256: $sha256, scan: $scan}' >>"$ROWS"
done

receipt="$(jq -cn \
  --arg source "$SOURCE" --argjson dirty "$DIRTY" --arg cargo_version "$CARGO_VERSION" \
  --argjson crates "$(jq -s '.' "$ROWS")" \
  '{source: $source, dirty: $dirty, cargo_version: $cargo_version, crates: $crates, all_pass: true}')"
echo "== inspection receipt: all crates packaged and scanned"
emit_receipt "$receipt"
