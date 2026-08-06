#!/usr/bin/env bash
#
# Regenerate the Homebrew tap formula for a published saya-cli release and push
# it to databook-studio/homebrew-tap.
#
# The release (with its per-platform archives and SHA256SUMS) must already exist
# for the given version. Checksums are read from the release's SHA256SUMS, so
# the formula always matches the exact published artifacts.
#
# Auth (push): HOMEBREW_TAP_TOKEN — a PAT with contents:write on the tap repo.
#   * In CI: set it as the HOMEBREW_TAP_TOKEN secret.
#   * The script no-ops in CI if the secret is unset, so it never fails a release
#     before the maintainer opts in.
#
# Usage:
#   ./scripts/update-homebrew-formula.sh 0.1.3
#   DRY_RUN=1 ./scripts/update-homebrew-formula.sh 0.1.3   # generate + diff, no push
set -euo pipefail

VERSION="${1:?usage: update-homebrew-formula.sh <version>   (e.g. 0.1.3)}"
DRY_RUN="${DRY_RUN:-0}"
TAP_REPO="databook-studio/homebrew-tap"
REL="https://github.com/databook-studio/saya-cli/releases/download/v${VERSION}"

if [ "${GITHUB_ACTIONS:-}" = "true" ] && [ "$DRY_RUN" != "1" ] && [ -z "${HOMEBREW_TAP_TOKEN:-}" ]; then
  echo "HOMEBREW_TAP_TOKEN is not set — skipping Homebrew formula bump."
  echo "Add it under Settings > Secrets and variables > Actions to enable this step."
  exit 0
fi

# Pull the published checksums so the formula matches the exact release artifacts.
sums=$(mktemp)
curl -fsSL "${REL}/SHA256SUMS" -o "$sums"
sha_for() {
  local hash
  hash=$(awk -v f="saya-${VERSION}-$1.tar.gz" '$2==f {print $1}' "$sums")
  [ -n "$hash" ] || { echo "no checksum for saya-${VERSION}-$1.tar.gz in SHA256SUMS" >&2; exit 1; }
  printf '%s' "$hash"
}
SHA_ARM_MAC=$(sha_for aarch64-apple-darwin)
SHA_INTEL_MAC=$(sha_for x86_64-apple-darwin)
SHA_LINUX=$(sha_for x86_64-unknown-linux-gnu)

# Clone the tap (token embedded for push; anonymous for a dry run).
work=$(mktemp -d)
if [ -n "${HOMEBREW_TAP_TOKEN:-}" ]; then
  git clone -q "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/${TAP_REPO}.git" "$work"
else
  git clone -q "https://github.com/${TAP_REPO}.git" "$work"
fi

mkdir -p "$work/Formula"
cat > "$work/Formula/saya.rb" <<EOF
class Saya < Formula
  desc "Database-aware terminal AI agent: TUI, schema discovery, read-only SQL"
  homepage "https://github.com/databook-studio/saya-cli"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "${REL}/saya-${VERSION}-aarch64-apple-darwin.tar.gz"
      sha256 "${SHA_ARM_MAC}"
    end
    on_intel do
      url "${REL}/saya-${VERSION}-x86_64-apple-darwin.tar.gz"
      sha256 "${SHA_INTEL_MAC}"
    end
  end

  on_linux do
    on_intel do
      url "${REL}/saya-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA_LINUX}"
    end
  end

  def install
    bin.install "saya"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/saya --version")
  end
end
EOF

cd "$work"
if git diff --quiet -- Formula/saya.rb; then
  echo "Formula already up to date for ${VERSION}."
  exit 0
fi

if [ "$DRY_RUN" = "1" ]; then
  echo "DRY_RUN — would commit and push this change:"
  git --no-pager diff -- Formula/saya.rb
  exit 0
fi

git -c user.name="saya-release-bot" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
  commit -q -m "saya ${VERSION}" -- Formula/saya.rb
git push -q origin HEAD:main
echo "Bumped Homebrew formula to ${VERSION}."
