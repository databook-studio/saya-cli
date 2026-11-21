//! `/permissions`: exactly the five facts, and `/grants` as its alias — the
//! same view, plus the alias's note once per session.

use saya_agent::{ApprovalPolicy, SessionGrants};

use super::{LABELS, PermissionsFacts, answer, panel_text};
use crate::chat::ops_status::Note;
use crate::interactive::session::permissions::grants;
use crate::interactive::session::state::SessionState;
use crate::slash::PermissionsRequest;

/// A session with a bound root, a composed host lane, sharing off, and the
/// given store and deny list — the five facts written from the fixtures.
fn state_with(host_composed: bool, sharing: bool) -> SessionState {
    let mut state = SessionState::new("test", None, "model");
    state.host_composed = host_composed;
    state.allow_data_sharing = sharing;
    state
}

fn seeded_store(tokens: &[&str]) -> SessionGrants {
    let store = SessionGrants::default();
    for token in tokens {
        store.grant(token);
    }
    store
}

/// The labels the view prints, in order: every line at the left margin is one
/// labelled fact, and nothing else is.
fn labels_of(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.starts_with(' '))
        .filter_map(|line| line.split_once(':'))
        .map(|(label, _)| label.to_owned())
        .collect()
}

/// The view states exactly the five facts the request names, in print order.
#[test]
fn the_view_states_exactly_the_five_facts() {
    let state = state_with(true, false);
    let store = seeded_store(&["sql:analytics", "workspace-write"]);
    let facts = PermissionsFacts::of_session(
        &state,
        ApprovalPolicy::Ask,
        &store,
        &["curl".to_owned(), "rm".to_owned()],
    );
    let text = panel_text(&facts);
    assert_eq!(
        labels_of(&text),
        LABELS.to_vec(),
        "exactly the five facts, in order:\n{text}"
    );
    assert!(
        text.starts_with("Mode: ask"),
        "the mode is the engine's own word: {text}"
    );
    // The grant store is the record, in its own words.
    assert!(
        text.contains(
            "session grants (die with this session): 2\n    sql:analytics\n    \
                      workspace-write"
        ),
        "the store's listing is nested under its label: {text}"
    );
    assert!(
        text.contains("Deny list: curl, rm"),
        "the deny list names the programs: {text}"
    );
    assert!(
        text.contains("Host lane: composed"),
        "the host lane is stated: {text}"
    );
    assert!(
        text.contains("Data sharing: off"),
        "data sharing is stated: {text}"
    );
    assert_eq!(
        text.lines().count(),
        8,
        "five labelled facts, with the store's two tokens and its header nested:\n{text}"
    );
}

/// Every fact states an absent value rather than a zero or an invention.
#[test]
fn an_empty_store_and_deny_list_state_what_is_absent() {
    let state = state_with(false, true);
    let store = SessionGrants::default();
    let facts = PermissionsFacts::of_session(&state, ApprovalPolicy::Ask, &store, &[]);
    let text = panel_text(&facts);
    assert_eq!(labels_of(&text), LABELS.to_vec(), "{text}");
    assert!(
        text.contains("Deny list: none — no --deny program refuses a call this session"),
        "an empty deny list says so: {text}"
    );
    assert!(
        text.contains("Host lane: not composed"),
        "an uncomposed lane says so: {text}"
    );
    assert!(
        text.contains("Data sharing: on"),
        "sharing is stated: {text}"
    );
    assert!(
        text.contains("(none — nothing pre-answers this session yet"),
        "an empty store keeps the listing's own empty state: {text}"
    );
}

/// Under bypass the store's listing still states the mode first — the words
/// the prompts offered — and the view's own Mode fact agrees with it.
#[test]
fn the_store_states_its_own_mode_under_bypass() {
    let state = state_with(true, false);
    let store = seeded_store(&["sql:analytics"]);
    let facts = PermissionsFacts::of_session(&state, ApprovalPolicy::Bypass, &store, &[]);
    let text = panel_text(&facts);
    assert!(
        text.contains("mode bypass: every call runs without asking"),
        "the listing's bypass line survives: {text}"
    );
    assert!(text.starts_with("Mode: bypass"), "{text}");
}

/// `/grants` is `/permissions` plus the alias's note, once per session; the
/// canonical spelling never carries a note at all.
#[test]
fn grants_is_the_view_plus_its_note_once() {
    let mut state = state_with(true, false);
    let store = seeded_store(&["sql:analytics"]);
    let canonical = panel_text(&PermissionsFacts::of_session(
        &state,
        ApprovalPolicy::Ask,
        &store,
        &[],
    ));

    let first = answer(
        &mut state,
        PermissionsRequest::grants(),
        ApprovalPolicy::Ask,
        &store,
        &[],
    );
    assert_eq!(
        first,
        format!("{canonical}\n{}", Note::Grants.text()),
        "the alias is the same view, then the note"
    );
    let second = answer(
        &mut state,
        PermissionsRequest::grants(),
        ApprovalPolicy::Ask,
        &store,
        &[],
    );
    assert_eq!(second, canonical, "the note is said once per session");

    for _ in 0..2 {
        let canonical = answer(
            &mut state,
            PermissionsRequest::permissions(),
            ApprovalPolicy::Ask,
            &store,
            &[],
        );
        assert_eq!(
            canonical,
            panel_text(&PermissionsFacts::of_session(
                &state,
                ApprovalPolicy::Ask,
                &store,
                &[]
            )),
            "/permissions states the five facts and no note"
        );
        assert!(!canonical.contains("note:"), "{canonical}");
    }
}

/// The two spellings cannot disagree about the store: the alias's view is the
/// canonical view byte for byte, the note aside.
#[test]
fn the_alias_and_the_command_render_the_same_view() {
    let store = seeded_store(&["workspace-write", "sql:analytics"]);
    let facts = PermissionsFacts::of_session(
        &state_with(true, false),
        ApprovalPolicy::Ask,
        &store,
        &["curl".to_owned()],
    );
    let mut alias_state = state_with(true, false);
    let alias = answer(
        &mut alias_state,
        PermissionsRequest::grants(),
        ApprovalPolicy::Ask,
        &store,
        &["curl".to_owned()],
    );
    assert!(
        alias.starts_with(&panel_text(&facts)),
        "the alias answers the one view: {alias}"
    );
    assert_eq!(
        facts.fact("Grants"),
        grants::listing(ApprovalPolicy::Ask, &store),
        "the Grants fact is the store's own listing, verbatim"
    );
}
