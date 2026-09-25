#!/usr/bin/env bash
#
# Verify that the Homebrew tap serves the release being cut.
#
# Reads Formula/saya.rb from databook-studio/homebrew-tap anonymously and
# compares the version in its release URLs against the version argument. Run
# this after scripts/update-homebrew-formula.sh, in the release workflow.
#
# Deliberately needs no HOMEBREW_TAP_TOKEN: the tap is public, and a check that
# required the tap token could never detect that token being broken — which is
# the failure this guard exists for. It clones instead of reading
# raw.githubusercontent.com because the raw CDN can serve a stale revision for
# minutes after the bump's push.
#
# Exit codes:
#   0  the tap serves exactly <version>
#   1  the tap is stale — it serves a different version, mixed versions, or no
#      formula at all. The message names the served and expected versions.
#   2  the check could not run — bad usage, the tap was unreachable, the
#      formula could not be read, or it carries no recognizable release URL.
#      Never reported as staleness; still fails, because an unverifiable
#      release is not a verified one.
#
# Env:
#   TAP_CHECK_WARN_ONLY=1     report a stale tap as a ::warning and exit 0.
#                             Used when HOMEBREW_TAP_TOKEN is deliberately
#                             unset (the documented bump no-op): staleness is
#                             then a channel that was opted out of, not a
#                             broken release.
#   SAYA_TAP_FORMULA_FILE=<f> read the formula from <f> instead of cloning the
#                             tap — test and offline-debugging seam, in the
#                             spirit of DRY_RUN.
#
# Usage:
#   ./scripts/check-homebrew-tap.sh 0.4.1
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: check-homebrew-tap.sh <version>   (e.g. 0.4.1)" >&2
  exit 2
fi
VERSION="$1"
TAP_REPO="databook-studio/homebrew-tap"
FORMULA="Formula/saya.rb"

fail_stale() {
  if [ "${TAP_CHECK_WARN_ONLY:-0}" = "1" ]; then
    echo "::warning::$*"
    exit 0
  fi
  echo "::error::$*" >&2
  exit 1
}

fail_unverifiable() {
  echo "::error::tap check could not run: $1" >&2
  exit 2
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

formula="$work/$FORMULA"
if [ -n "${SAYA_TAP_FORMULA_FILE:-}" ]; then
  formula="$SAYA_TAP_FORMULA_FILE"
else
  # Anonymous clone: the tap is public, and the clone mirrors the tokenless
  # path of update-homebrew-formula.sh. No CDN cache can serve a stale tip.
  git clone -q --depth 1 "https://github.com/${TAP_REPO}.git" "$work" \
    || fail_unverifiable "could not clone https://github.com/${TAP_REPO}.git (network failure, or the tap moved)"
fi

# The generated formula has no version stanza; the version it serves is the one
# embedded in its release URLs (.../releases/download/v<version>/...). Only
# `url` lines count, so a stale version quoted in a comment cannot make a
# correct formula look mixed.
if [ ! -f "$formula" ]; then
  fail_stale "the tap serves no ${FORMULA} at all; this release ${VERSION} never reached Homebrew. Checked ${formula}."
fi

if ! served=$(sed -n 's/^[[:space:]]*url ".*releases\/download\/v\([^/"]*\)\/.*/\1/p' "$formula" | LC_ALL=C sort -u); then
  fail_unverifiable "could not read ${FORMULA} to extract the version it serves (unreadable file?)."
fi
if [ -z "$served" ]; then
  fail_unverifiable "${FORMULA} carries no .../releases/download/v<version>/ URL, so the served version is unknowable."
fi

case "$served" in
  *$'\n'*)
    fail_stale "the tap serves mixed versions ($(printf '%s' "$served" | paste -sd, -)), but this release is ${VERSION}." \
      "The formula must carry a single version across all its URLs."
    ;;
esac

if [ "$served" != "$VERSION" ]; then
  fail_stale "the tap serves ${served}, but this release is ${VERSION} — brew install will keep handing out ${served}." \
    "Check the bump-homebrew job above: the formula push did not land."
fi

echo "Tap ${TAP_REPO} serves ${served}, which matches release ${VERSION}."