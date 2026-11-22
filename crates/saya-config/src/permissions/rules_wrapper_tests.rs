//! The deny side's reading of the launchers (SEC3): the programs that run
//! another program named after them, and the interpreters whose code the
//! reading cannot see.
//!
//! Split from `rules_deny_tests` because that file pins the SEC2 spellings and
//! is already near the reviewed length; the behaviour is the same matcher.

use super::{CommandDecision, CommandRule, CommandRules, decide};

fn rules(deny: &[&str]) -> CommandRules {
    CommandRules {
        deny: deny
            .iter()
            .map(|rule| CommandRule::parse(rule).expect("a valid test rule"))
            .collect(),
        ..Default::default()
    }
}

fn denied<'a>(rules: &'a CommandRules, command: &str) -> Option<&'a str> {
    match decide(rules, command, None) {
        CommandDecision::Deny(rule) => Some(rule.text()),
        _ => None,
    }
}

/// A launcher's own options, their values, and its terminator are not the
/// program: `timeout 5 rm`, `nice -n 5 rm`, `stdbuf -oL rm`, `xargs -I{} rm`,
/// and `find . -exec rm {} \;` all name `rm` exactly as `rm` does — and a
/// shell or `eval` script behind a launcher is read the same way.
#[test]
fn the_deny_side_reads_through_the_launchers_that_run_a_program() {
    let deny_rm = rules(&["rm"]);
    for command in [
        // timeout: its options, then the duration it waits, then the program.
        "timeout 5 rm -rf x",
        "timeout -s KILL 5 rm -rf x",
        "timeout --signal=KILL 5 rm -rf x",
        // nice: its adjustment, attached or as a value.
        "nice rm -rf x",
        "nice -n 5 rm -rf x",
        "nice -19 rm -rf x",
        // nohup and time.
        "nohup rm -rf x",
        "time rm -rf x",
        "time -p rm -rf x",
        // stdbuf: its buffering options, attached or as a value.
        "stdbuf -oL rm -rf x",
        "stdbuf -o L rm -rf x",
        "stdbuf -i0 -o0 -e0 rm -rf x",
        // xargs: its options, and the values of the ones that carry one.
        "xargs rm",
        "xargs -I{} rm {}",
        "xargs -n 1 rm",
        "xargs -P4 -L1 rm",
        "xargs -0 rm -rf x",
        "echo hi | xargs -I{} rm {}",
        // find: the program after -exec/-execdir/-ok, either terminator.
        "find . -exec rm -rf {} \\;",
        "find . -execdir rm -rf {} +",
        "find . -name x -ok rm {} \\;",
        // eval: its joined arguments are a script, read with the same reading.
        "eval rm -rf x",
        "eval \"rm -rf x\"",
        // A launcher in front of a shell's own script is read through both.
        "timeout 5 sh -c 'rm -rf x'",
        "xargs -I{} sh -c 'rm -rf x'",
        "nohup bash -c \"rm -rf x\"",
    ] {
        assert_eq!(
            denied(&deny_rm, command),
            Some("rm"),
            "must deny: {command}"
        );
    }
}

/// An interpreter's inline program is code the deny side cannot read, so it
/// reports itself undecided and a command naming a denied program anywhere in
/// it is refused rather than run — behind a launcher too, and inside `find`.
#[test]
fn an_interpreter_running_code_the_deny_side_cannot_read_is_refused() {
    let deny_rm = rules(&["rm"]);
    for command in [
        "python3 -c \"import os; os.system('rm -rf x')\"",
        "python -c \"import os; os.system('rm -rf x')\"",
        "perl -e 'system(\"rm -rf x\")'",
        "ruby -e 'system(\"rm -rf x\")'",
        "node -e 'require(\"child_process\").execSync(\"rm -rf x\")'",
        "awk 'BEGIN{system(\"rm -rf x\")}'",
        "gawk 'BEGIN{system(\"rm -rf x\")}'",
        "php -r 'system(\"rm -rf x\");'",
        "nice -n 5 python3 -c \"import os; os.system('rm -rf x')\"",
        "timeout 5 perl -e 'system(\"rm -rf x\")'",
        "xargs -I{} python3 -c \"import os; os.system('rm -rf x')\"",
        "find . -exec awk 'BEGIN{system(\"rm -rf x\")}' {} \\;",
    ] {
        assert_eq!(
            denied(&deny_rm, command),
            Some("rm"),
            "must deny: {command}"
        );
    }
}

/// Reading through the launchers widens nothing: a launcher running something
/// that is not the denied program still runs, an inline program that names no
/// denied program still runs, and a name that merely begins with the denied one
/// is not the denied one.
#[test]
fn the_launchers_do_not_refuse_what_they_do_not_run() {
    let deny_rm = rules(&["rm"]);
    for command in [
        "xargs echo",
        "xargs -I{} echo {}",
        "timeout 5 ls",
        "nice -n 5 ls",
        "nohup ls",
        "stdbuf -oL ls",
        "time ls",
        "find . -name x",
        "find . -name rm",
        "find . -exec echo {} \\;",
        "eval echo hi",
        "sh -c 'xargs echo'",
        "python3 -c 'print(1)'",
        "awk '{print $1}'",
        "python3 -c 'print(\"rmx\")'",
        "xargs rmx",
        "timeout 5 rmq",
        "find . -exec rmq {} \\;",
    ] {
        assert_eq!(denied(&deny_rm, command), None, "must not deny: {command}");
    }
}
