//! The tool-approval modal on the composed screen: it renders the shared fact
//! body — the same `call_facts` output the terminal prompt renders — so the
//! modal half of the parity property is pinned by snapshot.

use super::support::{empty_app, fixed_status, render_buffer};

// --- The tool-approval modal renders the shared fact body. -------------------

/// The approval modal renders the per-call fact body verbatim — the same
/// `call_facts` output the terminal prompt renders — plus the shared answers
/// line. This is the modal half of the parity property: the body the modal
/// shows is the body the terminal prompt shows, byte for byte.
#[test]
fn approval_modal_renders_the_shared_fact_body() {
    let mut app = empty_app();
    let tool = crate::interactive::session_definitions::http_fetch();
    let arguments = serde_json::json!({"url": "https://api.github.com/repos/x/y"});
    let facts = crate::approval_facts::ApprovalFacts {
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..crate::approval_facts::ApprovalFacts::default()
    };
    let grant = crate::grant_token::grant_token(&tool.name, &arguments, None, &facts);
    let detail = crate::approval_facts::call_facts(
        &tool.name,
        &arguments,
        grant.as_deref(),
        &facts,
        None,
        None,
    );
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(super::super::types::PendingApproval {
        tool: tool.name.clone(),
        detail,
        grant,
        scroll: 0,
        respond,
    });
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}
