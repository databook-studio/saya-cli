#!/usr/bin/env bash
# Compare the chat session against a control build on ref/06-plan.md §8's
# measures: run the ignored perf harness for both binaries on this machine and
# write target/perf/compare.md with the plan's pass bars applied.
#
#   scripts/perf-compare.sh <control-bin> <candidate-bin> [k]
#
# Both arms run on the same machine, one after the other, with the machine kept
# awake (caffeinate -i on macOS) so a sleeping display cannot skew a window.
# Nothing here talks to a paid provider: the harness uses the scripted mock on
# loopback. The timed campaign belongs on a quiet machine — this script says
# nothing about the numbers it collects beyond the bars the plan states.
set -euo pipefail

usage="usage: scripts/perf-compare.sh <control-bin> <candidate-bin> [k]"
control="${1:?$usage}"
candidate="${2:?$usage}"
k="${3:-5}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
perf_dir="$root/target/perf"
mkdir -p "$perf_dir"

for bin in "$control" "$candidate"; do
    if [[ ! -x "$bin" ]]; then
        echo "perf-compare: $bin is not an executable" >&2
        exit 2
    fi
done

# The harness runs with the crate directory as its working directory, so a
# relative arm would name a file that does not exist there.
absolute() {
    (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd)" "$(basename "$1")")
}
control="$(absolute "$control")"
candidate="$(absolute "$candidate")"

wake=()
if command -v caffeinate >/dev/null 2>&1; then
    wake=(caffeinate -i)
fi

run_arm() {
    local label="$1" bin="$2"
    echo "perf-compare: $label arm ($bin), k=$k" >&2
    rm -f "$perf_dir/$label.jsonl"
    SAYA_PERF_BIN="$bin" SAYA_PERF_K="$k" SAYA_PERF_LABEL="$label" \
        ${wake[@]+"${wake[@]}"} \
        cargo test --manifest-path "$root/Cargo.toml" -p saya-cli --locked \
        --test perf_chat -- --ignored --exact \
        the_chat_session_meets_the_plans_performance_measures --nocapture
}

run_arm control "$control"
run_arm candidate "$candidate"

python3 - "$perf_dir/control.jsonl" "$perf_dir/candidate.jsonl" \
    "$perf_dir/compare.md" "$control" "$candidate" "$k" <<'PY'
import json
import platform
import statistics
import sys

control_path, candidate_path, out_path, control_bin, candidate_bin, k = sys.argv[1:]


def load(path):
    with open(path) as handle:
        return [json.loads(line) for line in handle if line.strip()]


def median(records, key):
    values = [record[key] for record in records if record.get(key) is not None]
    return statistics.median(values) if values else None


def show(value, unit):
    return "skipped" if value is None else f"{value:.2f} {unit}"


control = load(control_path)
candidate = load(candidate_path)

# Each row states the plan's bar and how a value is judged against it. A row
# with no verdict is a measure the plan only asks to be reported, and a row the
# harness could not take says so instead of borrowing a number.
rows = [
    ("first message: send → first streamed event", "first_event_ms", "ms", "control + 10 %", "higher_is_worse", 1.10),
    ("steady turn overhead, p50", "overhead_p50_ms", "ms", "≤ 50 ms", "at_most", 50.0),
    ("steady turn overhead, p95", "overhead_p95_ms", "ms", "≤ 120 ms", "at_most", 120.0),
    ("request bytes per turn", "request_bytes_median", "bytes", "reported (GR causes only)", "reported", None),
    ("idle TUI CPU over the window", "idle_cpu_tui_pct", "%", "≤ 1 %", "at_most", 1.0),
    ("idle line-front CPU over the window", "idle_cpu_line_pct", "%", "reported", "reported", None),
    ("draws per second while streaming", "draws_per_second", "/s", "≤ 30/s", "at_most", 30.0),
    ("interval between frames", "frame_interval_ms", "ms", "≤ 2 ms each — not asserted", "unasserted", None),
    ("disk written per turn", "disk_bytes_per_turn", "bytes", "≤ 64 KiB (65536 B)", "at_most", 65536.0),
]

lines = [
    "# PERF compare — chat session",
    "",
    f"- control: `{control_bin}`",
    f"- candidate: `{candidate_bin}`",
    f"- runs per arm: {k} (medians below)",
    f"- machine: {platform.system()} {platform.machine()}, {platform.processor() or 'unknown cpu'}",
    "- bars: `ref/06-plan.md` §8 (PERF)",
    "",
    "| Measure | Control | Candidate | Delta | Bar | Verdict |",
    "| --- | --- | --- | --- | --- | --- |",
]

verdicts = []
for name, key, unit, bar, rule, limit in rows:
    control_value = median(control, key)
    candidate_value = median(candidate, key)
    if control_value is None or candidate_value is None:
        delta, verdict = "—", "not measured"
    else:
        change = candidate_value - control_value
        delta = f"{change:+.2f} {unit} ({change / control_value * 100:+.1f} %)" if control_value else "—"
        if rule == "at_most":
            verdict = "pass" if candidate_value <= limit else "FAIL"
        elif rule == "higher_is_worse":
            verdict = "pass" if candidate_value <= control_value * limit else "FAIL"
        elif rule == "unasserted":
            verdict = "not asserted (see REPORT.md)"
        else:
            verdict = "reported"
    verdicts.append(verdict)
    lines.append(
        f"| {name} | {show(control_value, unit)} | {show(candidate_value, unit)} "
        f"| {delta} | {bar} | {verdict} |"
    )

skipped = sorted({reason for record in control + candidate for reason in record.get("skipped", [])})
lines += ["", "## Notes", ""]
if skipped:
    lines += ["- skipped by the harness: " + "; ".join(skipped)]
lines += [
    "- `interval between frames` is what the pty stream can show without production",
    "  instrumentation: the milliseconds a single frame costs would need a timing",
    "  inside `Terminal::draw`, which the harness may not add. `draws per second`",
    "  is counted directly and is the bar that is asserted.",
    "- a `FAIL` on the first-message row is measured against the control arm's own",
    "  median on this machine, not against a stored number.",
]

with open(out_path, "w") as handle:
    handle.write("\n".join(lines) + "\n")

print(f"perf-compare: wrote {out_path}")
print("\n".join(lines))
failed = [row for row, verdict in zip(rows, verdicts) if verdict == "FAIL"]
if failed:
    print(f"\nperf-compare: {len(failed)} measure(s) outside the plan's bars", file=sys.stderr)
    sys.exit(1)
PY
