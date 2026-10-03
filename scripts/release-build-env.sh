#!/usr/bin/env bash
# Configure source-path remapping consistently for Cargo and native cc builds.

configure_saya_release_build_env() {
  local workspace="$1" cargo_home="${CARGO_HOME:-$HOME/.cargo}" flag
  case "${OSTYPE:-}" in
    msys*|cygwin*|win32*)
      command -v cygpath >/dev/null 2>&1 && {
        workspace="$(cygpath -w "$workspace")"
        cargo_home="$(cygpath -w "$cargo_home")"
      }
      ;;
  esac
  local -a rust_flags=()
  if [[ -n "${CARGO_ENCODED_RUSTFLAGS+x}" ]]; then
    if [[ -n "$CARGO_ENCODED_RUSTFLAGS" ]]; then
      local remaining="$CARGO_ENCODED_RUSTFLAGS"
      while [[ "$remaining" == *$'\x1f'* ]]; do
        rust_flags+=("${remaining%%$'\x1f'*}")
        remaining="${remaining#*$'\x1f'}"
      done
      rust_flags+=("$remaining")
    fi
  elif [[ -n "${RUSTFLAGS:-}" ]]; then
    IFS=$' \t\n' read -r -d '' -a rust_flags < <(printf '%s\0' "$RUSTFLAGS")
  fi
  rust_flags+=("--remap-path-prefix=$workspace=/saya" "--remap-path-prefix=$cargo_home=/cargo")
  local encoded=""
  for flag in "${rust_flags[@]}"; do
    encoded+="${encoded:+$'\x1f'}$flag"
  done
  export CARGO_ENCODED_RUSTFLAGS="$encoded"

  # cc 1.4.2 parses *FLAGS with shellword only when enabled. MSVC cl.exe has no
  # documented source-path remap equivalent, so keep its build to CI artifact scans.
  case "${OSTYPE:-}" in
    msys*|cygwin*|win32*) return ;;
  esac
  local cxx_encoded="${CXXFLAGS:-}"
  for flag in "-ffile-prefix-map=$workspace=/saya" "-ffile-prefix-map=$cargo_home=/cargo"; do
    printf -v flag '%q' "$flag"
    cxx_encoded+="${cxx_encoded:+ }$flag"
  done
  export CXXFLAGS="$cxx_encoded"
  export CC_SHELL_ESCAPED_FLAGS=1
}
