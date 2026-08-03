#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="${SAYA_VERSION:-$(sed -n '/^\[package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' crates/saya-cli/Cargo.toml | head -n 1)}"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
OUTPUT_DIR="${SAYA_PACKAGE_DIR:-$ROOT_DIR/dist}"
ARCHIVE="${OUTPUT_DIR}/saya-${VERSION}-${HOST}.tar.gz"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/saya-package.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT

cargo build --release --locked -p saya-cli

mkdir -p "$OUTPUT_DIR" "$WORK_DIR/saya-${VERSION}-${HOST}"
cp "target/release/saya" "$WORK_DIR/saya-${VERSION}-${HOST}/saya"
cp README.md LICENSE SECURITY.md "$WORK_DIR/saya-${VERSION}-${HOST}/"
tar -czf "$ARCHIVE" -C "$WORK_DIR" "saya-${VERSION}-${HOST}"
if command -v shasum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && shasum -a 256 "$(basename "$ARCHIVE")" > "$(basename "$ARCHIVE").sha256")
else
    (cd "$OUTPUT_DIR" && sha256sum "$(basename "$ARCHIVE")" > "$(basename "$ARCHIVE").sha256")
fi

if command -v shasum >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && shasum -a 256 -c "$(basename "$ARCHIVE").sha256")
else
    (cd "$OUTPUT_DIR" && sha256sum -c "$(basename "$ARCHIVE").sha256")
fi

SMOKE_DIR="$WORK_DIR/smoke"
mkdir "$SMOKE_DIR"
tar -xzf "$ARCHIVE" -C "$SMOKE_DIR"
EXTRACTED="$SMOKE_DIR/saya-${VERSION}-${HOST}/saya"
"$EXTRACTED" --version >/dev/null
(
    cd "$SMOKE_DIR"
    "$EXTRACTED" --non-interactive config doctor >/dev/null
)

echo "archive: $ARCHIVE"
echo "checksum: ${ARCHIVE}.sha256"
echo "signing: external credential and release-plan gate; no signing performed"
