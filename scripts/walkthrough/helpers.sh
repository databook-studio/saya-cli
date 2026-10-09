fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

step() {
  printf '\n== %s ==\n\n' "$*"
}

note() {
  printf '%s\n' "$*"
}

# run <expected-exit> <saya args...>: print the command, then its combined
# output, then require the exit code to be exactly <expected-exit>.
#
# The command line is echoed copy-paste-safe: an argument containing spaces or
# shell metacharacters is shown single-quoted, anything else verbatim.
display_arg() {
  case "$1" in
    "" | *[![:alnum:]%+,./:@^_-]*) printf "'%s'" "$1" ;;
    *) printf '%s' "$1" ;;
  esac
}

run() {
  local want="$1"
  shift
  printf '$ %s' "$SAYA_DISPLAY"
  local arg
  for arg in "$@"; do
    printf ' %s' "$(display_arg "$arg")"
  done
  printf '\n\n'
  local out rc
  out=$("${SAYA_PROG[@]}" "$@" 2>&1)
  rc=$?
  if [ -n "$out" ]; then
    printf '%s\n' "$out"
  fi
  if [ "$rc" -ne "$want" ]; then
    fail "\"$*\" exited $rc, expected $want"
  fi
  LAST_OUT="$out"
}

# require <needle>: the previous command's output had to contain <needle>.
require() {
  if [[ "$LAST_OUT" != *"$1"* ]]; then
    printf -- '--- output was ---\n%s\n' "$LAST_OUT" >&2
    fail "expected output to contain: $1"
  fi
}

# The journeys below run saya from INSIDE the sandbox: a project config layer
# (.saya/ in the checkout's cwd) would otherwise be consulted. In cargo-run
# mode the manifest path is pinned explicitly, so cargo still finds the
# workspace from the sandbox; the printed line keeps the reader-facing form.
REPO_DIR=$PWD
if [ -n "${SAYA_BIN:-}" ]; then
  RUN_PROG=("${SAYA_PROG[@]}")
else
  RUN_PROG=(cargo run -q --manifest-path "$REPO_DIR/Cargo.toml" -p saya-cli --)
fi

# run_from <dir> <expected-exit> <saya args...>: like run, but the command
# executes with <dir> as its working directory.
run_from() {
  local dir="$1" want="$2"
  shift 2
  printf '$ (cd %s) %s' "$dir" "$SAYA_DISPLAY"
  local arg
  for arg in "$@"; do
    printf ' %s' "$(display_arg "$arg")"
  done
  printf '\n\n'
  local out rc
  out=$(cd "$dir" && "${RUN_PROG[@]}" "$@" 2>&1)
  rc=$?
  if [ -n "$out" ]; then
    printf '%s\n' "$out"
  fi
  if [ "$rc" -ne "$want" ]; then
    fail "\"$*\" exited $rc, expected $want"
  fi
  LAST_OUT="$out"
}

# use_alice / use_bob: point the whole environment at one isolated root.
use_alice() {
  HOME="$ALICE_HOME"
  SAYA_CONFIG_HOME="$ALICE_HOME/config"
  SAYA_STATE_DB="$ALICE_HOME/state.sqlite3"
  SAYA_DEMO_DIR="$ALICE_HOME/demo"
  SAYA_INVESTIGATIONS_DIR="$ALICE_HOME/investigations"
  export HOME SAYA_CONFIG_HOME SAYA_STATE_DB SAYA_DEMO_DIR SAYA_INVESTIGATIONS_DIR
}

use_bob() {
  HOME="$BOB_HOME"
  SAYA_CONFIG_HOME="$BOB_HOME/config"
  SAYA_STATE_DB="$BOB_HOME/state.sqlite3"
  SAYA_DEMO_DIR="$BOB_HOME/demo"
  SAYA_INVESTIGATIONS_DIR="$BOB_HOME/investigations"
  export HOME SAYA_CONFIG_HOME SAYA_STATE_DB SAYA_DEMO_DIR SAYA_INVESTIGATIONS_DIR
}

step "0 · Environment"
note "Binary: $SAYA_DISPLAY (override with SAYA_BIN=…)"
