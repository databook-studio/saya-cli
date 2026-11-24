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
    return "not comparable" if value is None else f"{value:.2f} {unit}"


control = load(control_path)
candidate = load(candidate_path)

MEASURES = {
    "a": "first message: send → first streamed event",
    "b": "steady turn overhead beyond the scripted provider",
    "c": "request bytes per turn",
    "d": "idle CPU over the 60 s window (line + TUI)",
    "e": "draws while streaming 1 000 tokens",
    "f": "disk written per turn (job records + rewind)",
}

# Each row states the plan's bar, how a value is judged against it, and — for a
# measure an arm could not take — the words that arm's record uses for the
# reason. A row with no verdict is a measure the plan only asks to be reported;
# a row one arm could not take says `not comparable` with that arm's own reason
# instead of borrowing a number or disappearing.
rows = [
    ("a", "first message: send → first streamed event", "first_event_ms", "ms",
     "control + 10 %", "higher_is_worse", 1.10, None),
    ("b", "steady turn overhead, p50", "overhead_p50_ms", "ms", "≤ 50 ms", "at_most", 50.0, None),
    ("b", "steady turn overhead, p95", "overhead_p95_ms", "ms", "≤ 120 ms", "at_most", 120.0, None),
    ("c", "request bytes per turn", "request_bytes_median", "bytes",
     "reported (GR causes only)", "reported", None, None),
    ("d", "idle TUI CPU over the window", "idle_cpu_tui_pct", "%", "≤ 1 %", "at_most", 1.0,
     "idle CPU of the TUI"),
    ("d", "idle line-front CPU over the window", "idle_cpu_line_pct", "%", "reported",
     "reported", None, "idle CPU of the line front"),
    ("e", "draws per second while streaming", "draws_per_second", "/s", "≤ 30/s", "at_most",
     30.0, "draws while streaming"),
    ("e", "interval between frames", "frame_interval_ms", "ms",
     "≤ 2 ms each — not asserted", "unasserted", None, "draws while streaming"),
    ("f", "disk written per turn", "disk_bytes_per_turn", "bytes", "≤ 64 KiB (65536 B)",
     "at_most", 65536.0, None),
]


def reasons(records, needle):
    found = []
    for record in records:
        for skipped in record.get("skipped", []):
            if (needle is None or needle in skipped) and skipped not in found:
                found.append(skipped)
    return "; ".join(found)


def arm_spelling(records):
    flags = [record.get("posture_flag") for record in records if record.get("posture_flag")]
    plain = "yes" if any(record.get("plain_flag") for record in records) else "no"
    if not flags:
        return "unknown (the record does not state the spelling)"
    return f"`{flags[0]} read-only` (documents `--plain`: {plain})"


lines = [
    "# PERF compare — chat session",
    "",
    f"- control: `{control_bin}`",
    f"- candidate: `{candidate_bin}`",
    f"- runs per arm: {k} (medians below)",
    f"- machine: {platform.system()} {platform.machine()}, {platform.processor() or 'unknown cpu'}",
    "- bars: `ref/06-plan.md` §8 (PERF)",
    f"- control arguments: {arm_spelling(control)}",
    f"- candidate arguments: {arm_spelling(candidate)}",
    "",
    "| Measure | Control | Candidate | Delta | Bar | Verdict |",
    "| --- | --- | --- | --- | --- | --- |",
]

verdicts = []
coverage = {letter: [] for letter in MEASURES}
for letter, name, key, unit, bar, rule, limit, needle in rows:
    control_value = median(control, key)
    candidate_value = median(candidate, key)
    coverage[letter].append(control_value is not None and candidate_value is not None)
    if control_value is None or candidate_value is None:
        delta = "—"
        missing = []
        if control_value is None:
            missing.append("control")
        if candidate_value is None:
            missing.append("candidate")
        records = control if control_value is None else candidate
        why = reasons(records, needle) or "the arm reported no value"
        verdict = f"not comparable ({' and '.join(missing)}: {why})"
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

lines += [
    "",
    "## Coverage of the six measures",
    "",
    "| Measure | Both arms | Note |",
    "| --- | --- | --- |",
]
for letter, name in MEASURES.items():
    both = bool(coverage[letter]) and all(coverage[letter])
    note = "" if both else "not comparable — the rows above carry the reason"
    lines.append(f"| ({letter}) {name} | {'yes' if both else 'no'} | {note} |")

skipped = sorted({reason for record in control + candidate for reason in record.get("skipped", [])})
lines += ["", "## Notes", ""]
if skipped:
    lines += ["- skipped by the harness: " + "; ".join(skipped)]
lines += [
    "- the two arms state the same read-only posture under the spelling their own",
    "  `--help` documents; only the flag's name differs, the workload does not.",
    "- a measure one arm cannot take is stated in its row as `not comparable` with",
    "  that arm's own reason — it is never dropped from the table.",
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
