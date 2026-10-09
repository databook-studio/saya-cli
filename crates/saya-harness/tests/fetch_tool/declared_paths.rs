use super::*;

// --- beyond the required five: the positive paths and the declaration -------

/// A declared redirect chain is followed hop by hop — each hop re-judged —
/// and the final body is delivered, with the outcome naming the final URL.
#[tokio::test]
async fn follows_a_declared_redirect_and_delivers_the_final_body() {
    let (site, tool, _pulls) = fixture(
        vec![
            ("/start", redirect("/final")),
            ("/final", ok(vec![b"the redirected body"])),
        ],
        FetchLimits::default(),
    )
    .await;
    let outcome = tool
        .fetch("https://corpus.example.org/start")
        .await
        .expect("declared chain");
    assert_eq!(outcome.url, "https://corpus.example.org/final");
    assert_eq!(outcome.block.body, "the redirected body");
    assert!(!outcome.block.truncated);
    assert_eq!(
        site.served(),
        vec!["/start".to_owned(), "/final".to_owned()]
    );
}

/// The declaration is honest (DESIGN §6.3): an external side effect gated by
/// the run's approved fetch scope and this policy — not a per-call approval.
#[test]
fn the_tool_declares_its_effect_honestly() {
    let definition = http_fetch_definition();
    assert_eq!(definition.name, "http_fetch");
    assert!(!definition.read_only);
    assert!(definition.effect.external_side_effect);
    assert!(
        !definition.effect.requires_approval,
        "gating is scope + policy, not per-call prompts"
    );
    assert!(!definition.effect.database_data);
    assert!(
        definition
            .parameters
            .get("required")
            .is_some_and(|r| r == &serde_json::json!(["url"]))
    );
    let completion = definition.completion.as_deref().unwrap_or_default();
    assert!(!completion.is_empty());
    assert!(!completion.contains("failed"));
}
