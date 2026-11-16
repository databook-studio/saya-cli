use super::super::{completed_operator_result, stop_code_for};
use crate::render::RenderFormat;
use saya_harness::runtime::{NpmCheckOutcome, NpmCheckStatus, NpmInputBundleReceipt};

pub(super) fn check_node_result(
    outcome: &NpmCheckOutcome,
    stop: super::super::InstallStop,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    if !outcome.joined()
        || outcome.client_pid() == 0
        || outcome.cleanup() == saya_harness::runtime::CleanupStatus::Unknown
    {
        return crate::commands::failure_message(
            stop_code_for(stop, 5),
            "managed Node check did not return joined process and cleanup facts".to_owned(),
            format,
        );
    }
    let output_facts = output_facts(
        StreamOutputFacts {
            observed_bytes: outcome.stdout_total_bytes(),
            dropped_bytes: outcome.stdout_dropped_bytes(),
            truncated: outcome.stdout_truncated(),
        },
        StreamOutputFacts {
            observed_bytes: outcome.stderr_total_bytes(),
            dropped_bytes: outcome.stderr_dropped_bytes(),
            truncated: outcome.stderr_truncated(),
        },
        outcome.output_truncated(),
        outcome.output_text_may_be_lossy(),
    );
    let message = format!(
        "Managed Node check {:?} ({:?}): source {}, manifest {}, lock {}, input {}, bundle {}, effective toolchain {}, elapsed {} ms, output {} bytes (sha256 {}).\n{}\nRetained display text (not original bytes):\nstdout:\n{}\nstderr:\n{}",
        outcome.action(),
        outcome.status(),
        outcome.source_sha256(),
        outcome.manifest_sha256(),
        outcome.lock_sha256(),
        outcome.input_key_sha256(),
        outcome.bundle_sha256(),
        outcome.effective_toolchain_id(),
        outcome.elapsed_ms(),
        outcome.output_bytes(),
        outcome.output_sha256(),
        output_facts,
        terminal_safe_output(outcome.stdout()),
        terminal_safe_output(outcome.stderr()),
    );
    if outcome.status() == NpmCheckStatus::Succeeded
        && outcome.exit_code() == Some(0)
        && !outcome.timed_out()
        && !outcome.cancelled()
        && outcome.signal().is_none()
        && !outcome.killed_orphans()
        && outcome.cleanup() != saya_harness::runtime::CleanupStatus::Unknown
    {
        return completed_operator_result(message, stop, format);
    }
    if outcome.status() == NpmCheckStatus::Succeeded {
        return crate::commands::failure_message(
            stop_code_for(stop, 5),
            "managed Node check reported success without complete process cleanup facts".to_owned(),
            format,
        );
    }
    crate::commands::failure_message(
        stop_code_for(
            stop,
            match outcome.status() {
                NpmCheckStatus::TimedOut => 124,
                NpmCheckStatus::Cancelled => 130,
                NpmCheckStatus::Failed => outcome.exit_code().unwrap_or(1).clamp(1, 125),
                NpmCheckStatus::Succeeded => 5,
            },
        ),
        message,
        format,
    )
}

#[derive(Clone, Copy)]
struct StreamOutputFacts {
    observed_bytes: u64,
    dropped_bytes: u64,
    truncated: bool,
}

fn output_facts(
    stdout: StreamOutputFacts,
    stderr: StreamOutputFacts,
    output_truncated: bool,
    display_may_be_lossy: bool,
) -> String {
    format!(
        "stdout observed {} bytes, dropped {}, truncated {}; stderr observed {} bytes, dropped {}, truncated {}; combined output truncated {output_truncated}; display may be lossy {display_may_be_lossy}",
        stdout.observed_bytes,
        stdout.dropped_bytes,
        stdout.truncated,
        stderr.observed_bytes,
        stderr.dropped_bytes,
        stderr.truncated,
    )
}

fn terminal_safe_output(output: &str) -> String {
    let mut visible = String::with_capacity(output.len());
    for character in output.chars() {
        if character == '\n' || character == '\t' {
            visible.push(character);
        } else if character.is_control() {
            use std::fmt::Write as _;
            let _ = write!(visible, "\\u{{{:x}}}", character as u32);
        } else {
            visible.push(character);
        }
    }
    visible
}

pub(super) fn npm_input_message(receipt: &NpmInputBundleReceipt) -> String {
    format!(
        "Stored verified npm tarball inputs for lock {}: {} tarballs ({} bytes), bundle {}, toolchain {}. No project node_modules were installed.",
        receipt.lock_sha256(),
        receipt.tarball_count(),
        receipt.tarball_bytes(),
        receipt.bundle_sha256(),
        receipt.effective_toolchain_id(),
    )
}

#[cfg(test)]
mod tests {
    use super::{StreamOutputFacts, output_facts};

    #[test]
    fn output_description_keeps_raw_stream_bounds_and_lossy_display_explicit() {
        let facts = output_facts(
            StreamOutputFacts {
                observed_bytes: 80,
                dropped_bytes: 16,
                truncated: true,
            },
            StreamOutputFacts {
                observed_bytes: 24,
                dropped_bytes: 0,
                truncated: false,
            },
            true,
            true,
        );
        assert!(facts.contains("stdout observed 80 bytes, dropped 16, truncated true"));
        assert!(facts.contains("stderr observed 24 bytes, dropped 0, truncated false"));
        assert!(facts.contains("combined output truncated true"));
        assert!(facts.contains("display may be lossy true"));
    }
}
