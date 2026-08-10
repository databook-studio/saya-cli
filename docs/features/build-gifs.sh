#!/usr/bin/env bash
# Record the 0.2.0 feature GIFs and produce optimized, share-ready versions.
#
#   docs/features/build-gifs.sh            # record everything + optimize
#   docs/features/build-gifs.sh optimize   # only re-optimize existing raw GIFs
#
# Offline tapes are deterministic. The live tapes need the databook containers
# up and a Vivanti gateway key in .env.saya; the gateway (glm-5.2) reasons for
# ~20-55s, so those raw recordings are long — we speed them up here so the
# published clip plays in ~12-15s.
#
# Requires: vhs, ffmpeg. Run from the repo root.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Throwaway local dev passwords for the databook containers (username == password).
export SAYA_DOCKER_POSTGRES_PASSWORD="${SAYA_DOCKER_POSTGRES_PASSWORD:-databook}"
export SAYA_DOCKER_MYSQL_PASSWORD="${SAYA_DOCKER_MYSQL_PASSWORD:-databook}"

OFFLINE=(feat-splash feat-sql-explain feat-export)
LIVE=(feat-ask-sql-visibility feat-cross-database feat-sessions-resume)

# Speed factor per clip (macOS bash 3.2 has no associative arrays, so use a case).
# Offline clips are already tight (1x); the live ones are sped up.
speed_for() {
  case "$1" in
    feat-ask-sql-visibility) echo 3.5 ;;
    feat-cross-database)     echo 4   ;;
    feat-sessions-resume)    echo 1.4 ;;
    *)                       echo 1   ;;
  esac
}
WIDTH=1000

record() { echo "· recording $1"; vhs "docs/features/$1.tape" >/dev/null 2>&1; }

optimize() {
  local name="$1" speed src; speed="$(speed_for "$1")"; src="docs/features/$1.gif"
  local out="docs/features/$1.opt.gif" pal; pal="$(mktemp -t "$1-XXXX").png"
  [ -f "$src" ] || { echo "  ! $src missing, skipping"; return; }
  local vf="setpts=PTS/${speed},fps=15,scale=${WIDTH}:-1:flags=lanczos"
  ffmpeg -y -i "$src" -vf "${vf},palettegen=stats_mode=diff" "$pal" >/dev/null 2>&1
  ffmpeg -y -i "$src" -i "$pal" \
    -lavfi "${vf} [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=3" \
    "$out" >/dev/null 2>&1
  rm -f "$pal"
  # Publish the optimized clip over the raw one.
  mv "$out" "$src"
  echo "  ✓ $src  ($(du -h "$src" | cut -f1), ${speed}x)"
}

if [ "${1:-all}" != "optimize" ]; then
  for t in "${OFFLINE[@]}"; do record "$t"; done
  for t in "${LIVE[@]}";    do record "$t"; done
fi
for t in "${OFFLINE[@]}" "${LIVE[@]}"; do optimize "$t"; done
echo "done — GIFs in docs/features/"
