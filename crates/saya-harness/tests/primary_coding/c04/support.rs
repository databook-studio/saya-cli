use super::super::support;
use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{
        CheckDiagnostics, CheckExecutionInput, CheckInvocation, execute_check_with_diagnostics,
    },
    commands::CommandOwner,
};
use saya_types::CheckDiagnosticFormat;
use std::num::NonZeroU64;

pub(super) fn invocation(
    format: CheckDiagnosticFormat,
    malformed: bool,
    toolchain: Option<&str>,
) -> CheckInvocation {
    let kind = match format {
        CheckDiagnosticFormat::RustcJson => "rustc",
        CheckDiagnosticFormat::CargoJson => "cargo",
        _ => panic!("fixture format must be explicit"),
    };
    let mut argv = vec![kind.to_owned()];
    if malformed {
        argv.push("malformed".into());
    }
    CheckInvocation::new(
        "c04-profile".into(),
        "c04-check".into(),
        support::DIAGNOSTIC_HELPER.into(),
        argv,
        NonZeroU64::new(10).unwrap(),
        toolchain.map(str::to_owned),
        "confined".into(),
    )
    .unwrap()
    .with_diagnostic_format(format)
}

pub(super) async fn run(
    fixture: &support::Fixture,
    owner: &CommandOwner,
    invocation: &CheckInvocation,
) -> CheckDiagnostics {
    let ownership = fixture.ownership();
    execute_check_with_diagnostics(CheckExecutionInput {
        ownership: &ownership,
        authority: fixture.authority(),
        invocation,
        owner,
        cancellation: &CancellationToken::new(),
        program_dir: fixture.commands.program_dir(),
        output_root: &fixture.output_root,
        resolver: fixture.resolver(),
    })
    .await
    .expect("real confined diagnostic check must settle")
}
