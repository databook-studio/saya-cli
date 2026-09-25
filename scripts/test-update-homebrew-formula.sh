#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/update-homebrew-formula.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

BIN="$TEST_ROOT/bin"
mkdir -p "$BIN"

cat > "$BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf 'curl argv: %s\n' "$*" >> "$SAYA_LOG"
output=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then
    output="$2"
    shift 2
  else
    shift
  fi
done
[ -n "$output" ]
cat > "$output" <<'SUMS'
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  saya-0.4.1-aarch64-apple-darwin.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  saya-0.4.1-x86_64-apple-darwin.tar.gz
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  saya-0.4.1-x86_64-unknown-linux-gnu.tar.gz
SUMS
EOF

cat > "$BIN/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf 'git argv: %s\n' "$*" >> "$SAYA_LOG"
if [ -n "${GIT_ASKPASS:-}" ]; then
  printf 'askpass: %s\n' "$GIT_ASKPASS" >> "$SAYA_PATH_LOG"
  [ "$("$GIT_ASKPASS" 'Password')" = "${HOMEBREW_TAP_TOKEN:?}" ]
fi

if [ "${1:-}" = "--no-pager" ]; then
  shift
fi
while [ "${1:-}" = "-c" ]; do
  shift 2
done
case "${1:-}" in
  clone)
    shift
    [ "${1:-}" = "-q" ] && shift
    clone_url="${1:?missing clone URL}"
    destination="${2:?missing clone destination}"
    printf '%s\n' "$destination" > "$SAYA_CLONE_PATH"
    mkdir -p "$destination/Formula" "$destination/.git"
    cat > "$destination/.git/config" <<CONFIG
[remote "origin"]
    url = $clone_url
    fetch = +refs/heads/*:refs/remotes/origin/*
CONFIG
    cat > "$destination/Formula/saya.rb" <<'FORMULA'
class Saya < Formula
  url "https://github.com/databook-studio/saya-cli/releases/download/v0.4.0/saya-0.4.0-x86_64-unknown-linux-gnu.tar.gz"
end
FORMULA
    ;;
  diff)
    if [ "${SAYA_DIFF_MODE:?}" = changed ] && [[ "$*" == *--quiet* ]]; then
      exit 1
    fi
    exit 0
    ;;
  commit)
    printf 'git config: ' >> "$SAYA_LOG"
    cat "$(cat "$SAYA_CLONE_PATH")/.git/config" >> "$SAYA_LOG"
    ;;
  push)
    printf 'git config: ' >> "$SAYA_LOG"
    cat "$(cat "$SAYA_CLONE_PATH")/.git/config" >> "$SAYA_LOG"
    if [ "${SAYA_FAIL_GIT:-0}" = push ]; then
      echo 'stub push failed' >&2
      exit 1
    fi
    ;;
  *)
    echo "unexpected git invocation: $*" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$BIN/curl" "$BIN/git"

assert_absent() {
  local needle="$1" file="$2"
  if grep -Fq -- "$needle" "$file"; then
    echo "sentinel persisted in $file" >&2
    return 1
  fi
}

run_case() {
  local name="$1" expected_status="$2" diff_mode="$3" dry_run="$4" fail_git="$5"
  local case_root="$TEST_ROOT/$name" tmpdir="$TEST_ROOT/$name/tmp" log="$TEST_ROOT/$name/log" paths="$TEST_ROOT/$name/paths" clone_path="$TEST_ROOT/$name/clone-path"
  mkdir -p "$tmpdir"
  : > "$log"
  : > "$paths"

  set +e
  output=$(env \
    PATH="$BIN:$PATH" \
    TMPDIR="$tmpdir" \
    SAYA_LOG="$log" \
    SAYA_PATH_LOG="$paths" \
    SAYA_CLONE_PATH="$clone_path" \
    SAYA_DIFF_MODE="$diff_mode" \
    SAYA_FAIL_GIT="$fail_git" \
    HOMEBREW_TAP_TOKEN='sentinel-token-for-homebrew-test' \
    DRY_RUN="$dry_run" \
    bash "$SCRIPT" 0.4.1 2>&1
  )
  status=$?
  set -e

  if [ "$status" -ne "$expected_status" ]; then
    echo "$name: expected status $expected_status, got $status" >&2
    echo "$output" >&2
    return 1
  fi

  assert_absent 'sentinel-token-for-homebrew-test' "$log"
  assert_absent 'sentinel-token-for-homebrew-test' "$paths"
  assert_absent 'sentinel-token-for-homebrew-test' <(printf '%s\n' "$output")
  if find "$tmpdir" -mindepth 1 -print -quit | grep -q .; then
    echo "$name: temporary credential/checksum/work files remain" >&2
    find "$tmpdir" -mindepth 1 -print >&2
    return 1
  fi
}

run_case normal 0 changed 0 0
run_case noop 0 noop 0 0
run_case dry-run 0 changed 1 0
run_case failure 1 changed 0 push

echo "update-homebrew-formula credential handling contract valid"
