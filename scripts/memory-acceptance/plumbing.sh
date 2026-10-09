# -- Plumbing self-test --------------------------------------------------------
#
# The scorer cases above prove the oracle; these prove the run plumbing's
# decisions: which captured stream is an environment failure rather than a
# model verdict, which wall time is a probable cache hit, and that per-run
# nonces differ (independent samples). All pure: synthetic captures, no model,
# gateway, database, or built binary.

PLUMB_SCRATCH=""

case_http_401_stderr_is_an_environment_failure() {
  printf 'provider request failed: HTTP 401: authentication failed\n' \
    > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_a_benign_diagnostic_is_not_an_environment_failure() {
  printf '{"event":"diagnostic","message":"schema cache is warm"}\n' \
    > "$PLUMB_SCRATCH/err"
  printf '{"event":"assistant_text","text":"OK"}\n' > "$PLUMB_SCRATCH/out"
  ! environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_ignored_settings_warning_is_an_environment_failure() {
  printf '%s\n' \
    "warning: ignored 4 security-critical settings from this project's .saya/config.toml — run \`saya config doctor\` for which, and why." \
    > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_provider_not_configured_is_an_environment_failure() {
  printf 'provider is not configured: no api_key reference\n' > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_error_event_on_stdout_is_an_environment_failure() {
  : > "$PLUMB_SCRATCH/err"
  printf '%s\n' \
    '{"event":"error","message":"provider request failed: HTTP 401: authentication failed"}' \
    > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_cache_threshold_classifies_wall_times() {
  is_probable_cache_hit 0.42 || return 1
  is_probable_cache_hit 0.99 || return 1
  if is_probable_cache_hit 1.00; then return 1; fi
  if is_probable_cache_hit 12.34; then return 1; fi
}

case_run_nonces_differ_and_match_their_shape() {
  local one two
  one="$(new_run_nonce 1)"
  two="$(new_run_nonce 1)"
  # The property that defeats the cache: two draws for the SAME index differ,
  # so prompts never repeat across script invocations.
  [ -n "$one" ] && [ -n "$two" ] && [ "$one" != "$two" ] || return 1
  printf '%s\n%s\n' "$one" "$two" | grep -Eq '^run-[0-9]+-[0-9]+-[0-9]+$'
}

plumbing_test() {
  local scratch fails=0 total=0 idx
  scratch="$(mktemp -d)" \
    || { echo "✗ plumbing self-test could not create a scratch dir" >&2; return 2; }
  PLUMB_SCRATCH="$scratch"

  local -a names labels
  names=(
    case_http_401_stderr_is_an_environment_failure
    case_a_benign_diagnostic_is_not_an_environment_failure
    case_ignored_settings_warning_is_an_environment_failure
    case_provider_not_configured_is_an_environment_failure
    case_error_event_on_stdout_is_an_environment_failure
    case_cache_threshold_classifies_wall_times
    case_run_nonces_differ_and_match_their_shape
  )
  labels=(
    "HTTP 401 on the preflight stderr is an environment failure"
    "a benign diagnostic is not an environment failure"
    "the ignored security-critical settings warning is an environment failure"
    "a not-configured provider is an environment failure"
    "an error event on the preflight stdout is an environment failure"
    "cache-hit threshold: 0.42 yes, 1.00 no, 12.34 no"
    "run nonces: same-index draws differ, shape run-<i>-<r>-<r>"
  )
  for idx in "${!names[@]}"; do
    total=$((total + 1))
    if "${names[$idx]}"; then
      printf 'ok  plumbing: %s\n' "${labels[$idx]}"
    else
      printf 'FAIL plumbing: %s\n' "${labels[$idx]}"
      fails=$((fails + 1))
    fi
  done
  rm -rf "$scratch"
  PLUMB_SCRATCH=""
  echo
  if [ "$fails" -eq 0 ]; then
    printf '✓ %d/%d plumbing cases pass\n' "$total" "$total"
    return 0
  fi
  printf '✗ %d/%d plumbing cases fail\n' "$fails" "$total"
  return 1
}
