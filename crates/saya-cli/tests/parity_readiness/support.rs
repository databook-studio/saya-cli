pub use super::native::Fixture;
use saya_agent::CancellationToken;
use saya_cli::CodingApplication;
use serde_json::Value;
use std::sync::Arc;

pub async fn application(fixture: &Fixture) -> Arc<CodingApplication> {
    let runtime = fixture.runtime.clone();
    CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

pub fn blocked(report: &Value) -> Vec<&str> {
    report["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect()
}

pub fn optional(report: &Value) -> Vec<&str> {
    report["optional"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect()
}
