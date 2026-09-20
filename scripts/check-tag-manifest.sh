#!/usr/bin/env bash
#
# Refuse a release whose pushed tag disagrees with the release manifest.
#
# Compares the tag's version (GITHUB_REF_NAME with the leading "v" already
# stripped by the caller) against the manifest version from
# crates/saya-cli/Cargo.toml. On a workflow_dispatch validation build there
# is no tag, so an empty tag argument passes unchanged — the validation build
# keeps working exactly as it does today.
#
# Exit codes:
#   0  the tag matches the manifest, or there is no tag to check
#   1  the pushed tag disagrees with the manifest — the message names both
#   2  the check could not run — bad usage. Never reported as a mismatch.
#
# Usage:
#   ./scripts/check-tag-manifest.sh <tag-version> <manifest-version>
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: check-tag-manifest.sh <tag-version> <manifest-version>" >&2
  exit 2
fi
TAG_VERSION="$1"
MANIFEST_VERSION="$2"

if [ -z "${TAG_VERSION}" ]; then
  echo "No tag to check (workflow_dispatch validation build); tag/manifest guard passes."
  exit 0
fi

if [ -z "${MANIFEST_VERSION}" ]; then
  echo "::error::tag/manifest guard could not run: the manifest version is empty." >&2
  exit 2
fi

if [ "${TAG_VERSION}" != "${MANIFEST_VERSION}" ]; then
  echo "::error::refusing release: pushed tag v${TAG_VERSION} disagrees with manifest version ${MANIFEST_VERSION}. Push tag v${MANIFEST_VERSION}, or bump the manifest to ${TAG_VERSION}." >&2
  exit 1
fi

echo "Pushed tag v${TAG_VERSION} matches manifest version ${MANIFEST_VERSION}."
