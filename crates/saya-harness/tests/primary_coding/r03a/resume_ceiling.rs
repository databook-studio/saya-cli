use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::LocalOpenAi,
};
use saya_agent::{
    CancellationToken, EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome,
    EffectSettlement, PhysicalAttempt,
};
use saya_harness::coding::{
    budget::{BudgetSnapshot, JobBudgetLedger, UsageCoverage},
    coordinator::{child_event_channel, run_child_budgeted},
};
use saya_store::SqliteStateStore;
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy)]
enum ProviderUsage {
    Complete(u64, u64),
    PartialInput(u64),
    Missing,
    LegacyMissingSpan,
}

struct Case {
    name: &'static str,
    usage: ProviderUsage,
    ceiling: Option<u64>,
    expected_usage: Option<(u64, u64)>,
    should_admit: bool,
}

#[tokio::test]
async fn resumed_job_ceiling_uses_only_complete_durable_provider_usage() {
    let cases = [
        Case {
            name: "resume-at-ceiling",
            usage: ProviderUsage::Complete(50, 50),
            ceiling: Some(100),
            expected_usage: Some((50, 50)),
            should_admit: false,
        },
        Case {
            name: "resume-below-ceiling",
            usage: ProviderUsage::Complete(40, 40),
            ceiling: Some(100),
            expected_usage: Some((40, 40)),
            should_admit: true,
        },
        Case {
            name: "resume-ceiling-unset",
            usage: ProviderUsage::Complete(50, 50),
            ceiling: None,
            expected_usage: Some((50, 50)),
            should_admit: true,
        },
        Case {
            name: "resume-partial-usage",
            usage: ProviderUsage::PartialInput(50),
            ceiling: Some(100),
            expected_usage: None,
            should_admit: true,
        },
        Case {
            name: "resume-missing-usage",
            usage: ProviderUsage::Missing,
            ceiling: Some(100),
            expected_usage: None,
            should_admit: true,
        },
        Case {
            name: "resume-legacy-missing-span",
            usage: ProviderUsage::LegacyMissingSpan,
            ceiling: Some(100),
            expected_usage: None,
            should_admit: true,
        },
    ];

    for case in cases {
        let fixture = Fixture::new(case.name).await;
        let ledger = if matches!(case.usage, ProviderUsage::LegacyMissingSpan) {
            record_legacy_missing_span(&fixture, case.ceiling).await
        } else {
            record_provider_usage(&fixture, case.ceiling, case.usage).await
        };
        let page = fixture
            .store
            .list_coding_transport_attempts(&fixture.job, None, 100)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1, "{}", case.name);
        let record = &page.items[0];
        assert!(record.settlement.is_some(), "{}", case.name);
        match case.usage {
            ProviderUsage::Complete(input, output) => {
                let span = record.span.as_ref().expect("provider call writes its span");
                assert!(span.data().usage_complete, "{}", case.name);
                assert_eq!(span.data().reported_input_tokens, Some(input));
                assert_eq!(span.data().reported_output_tokens, Some(output));
            }
            ProviderUsage::PartialInput(_) | ProviderUsage::Missing => {
                let span = record.span.as_ref().expect("provider call writes its span");
                assert!(!span.data().usage_complete, "{}", case.name);
                assert_eq!(span.data().reported_input_tokens, None);
                assert_eq!(span.data().reported_output_tokens, None);
            }
            ProviderUsage::LegacyMissingSpan => assert!(record.span.is_none()),
        }

        fixture.store.close().await;
        drop(ledger);
        let store = SqliteStateStore::new(fixture.database.clone());
        let resumed = JobBudgetLedger::resume(store, fixture.job.clone(), job_limits())
            .await
            .unwrap();
        resumed.set_reported_token_ceiling(case.ceiling).await;
        let root = resumed.root_view();
        let admission = root.admit_model_request().await;
        if case.should_admit {
            let permit = admission.expect("resumed usage should allow this case");
            root.settle(
                permit,
                EffectSettlement::ModelRequest {
                    outcome: EffectOutcome::NotStarted,
                    reported_usage: None,
                    usage_complete: false,
                },
            )
            .await
            .unwrap();
        } else {
            assert!(
                matches!(admission, Err(EffectAdmissionError::Denied)),
                "complete usage at the configured ceiling must deny after resume"
            );
        }
        assert_usage(&resumed.snapshot().await, case.expected_usage, case.usage);
        drop(root);
        drop(resumed);
        fixture.close().await;
    }
}

async fn record_provider_usage(
    fixture: &Fixture,
    ceiling: Option<u64>,
    usage: ProviderUsage,
) -> JobBudgetLedger {
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(ceiling).await;
    let authority = fixture.add_child("agent-resume", "invocation-resume").await;
    let spec = fixture.spec(authority, "return one answer", 1, 1);
    let (events, _received) = child_event_channel(&spec);
    let factory = OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats: Arc::new(FactoryStats::default()),
        cancel_on_build: None,
        retry_delays: Vec::new(),
    };
    let task = run_child_budgeted(spec, &factory, events, CancellationToken::new(), &ledger);
    tokio::pin!(task);
    let request = tokio::select! {
        request = server.request() => request,
        _ = &mut task => panic!("child completed before its provider request"),
    };
    assert_eq!(request, Some(1));
    server.respond_sse(&response(usage));
    let output = tokio::time::timeout(Duration::from_secs(5), &mut task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.answer, "completed");
    assert_eq!(server.count.load(std::sync::atomic::Ordering::SeqCst), 1);
    server.shutdown().await;
    ledger.clone()
}

async fn record_legacy_missing_span(fixture: &Fixture, ceiling: Option<u64>) -> JobBudgetLedger {
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(ceiling).await;
    let root = ledger.root_view();
    let model = root.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let send = root.admit_physical_attempt(attempt).await.unwrap();
    root.settle(
        send,
        EffectSettlement::PhysicalAttempt {
            attempt,
            outcome: EffectOutcome::Succeeded,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    root.settle(
        model,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Succeeded,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    ledger
}

fn response(usage: ProviderUsage) -> String {
    let answer = "data: {\"choices\":[{\"delta\":{\"content\":\"completed\"},\"finish_reason\":\"stop\"}]}\n\n";
    let usage = match usage {
        ProviderUsage::Complete(input, output) => format!(
            "data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{input},\"completion_tokens\":{output}}}}}\n\n"
        ),
        ProviderUsage::PartialInput(input) => {
            format!("data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{input}}}}}\n\n")
        }
        ProviderUsage::Missing | ProviderUsage::LegacyMissingSpan => String::new(),
    };
    format!("{answer}{usage}data: [DONE]\n\n")
}

fn assert_usage(snapshot: &BudgetSnapshot, expected: Option<(u64, u64)>, usage: ProviderUsage) {
    let expected_input = expected.map(|(input, _)| input);
    let expected_output = expected.map(|(_, output)| output);
    assert_eq!(snapshot.reported_input_tokens, expected_input);
    assert_eq!(snapshot.reported_output_tokens, expected_output);
    if matches!(
        usage,
        ProviderUsage::PartialInput(_) | ProviderUsage::Missing | ProviderUsage::LegacyMissingSpan
    ) {
        assert_eq!(snapshot.usage_coverage, UsageCoverage::Partial);
    }
}

fn job_limits() -> saya_harness::coding::budget::CountLimits {
    limits(10, 10, 10, 4, 3, Duration::from_secs(30))
}
