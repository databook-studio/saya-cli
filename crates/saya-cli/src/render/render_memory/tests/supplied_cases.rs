/// The three outcomes render distinguishably (spec §5).
#[test]
fn the_three_outcomes_render_distinguishably() {
    let off = knowledge_supplied_text(KnowledgeOutcome::Off, &[], 0);
    let skipped = knowledge_supplied_text(KnowledgeOutcome::Skipped, &[], 0);
    let ran_empty = knowledge_supplied_text(ran(false), &[], 0);
    let ran_found = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                "c-1",
                "default_time_column",
                "created_at",
                Some("created_at"),
                ClaimStatus::Confirmed,
            )],
        )],
        0,
    );
    assert_eq!(off, "memory off · recall disabled\n");
    assert_eq!(
        skipped,
        "memory skipped · not permitted to read saved claims\n"
    );
    // Ran-and-found-nothing is silent (the decision), distinct from Off and
    // Skipped which each print a line — so the three states read differently.
    assert_eq!(ran_empty, "");
    assert!(ran_found.starts_with("memory supplied · 1 claim"));
    assert!(
        !ran_found.contains("1 claims"),
        "a single claim must not read as a plural: {ran_found}"
    );
    // All three states are mutually distinguishable on the rendered output.
    assert_ne!(off, skipped);
    assert_ne!(off, ran_empty);
    assert_ne!(skipped, ran_empty);
    assert_ne!(ran_empty, ran_found);
}

/// A candidate claim is marked, both in the count and on its line (spec §5).
#[test]
fn a_candidate_claim_is_marked() {
    let text = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![
                claim(
                    "c-1",
                    "default_time_column",
                    "created_at",
                    Some("created_at"),
                    ClaimStatus::Confirmed,
                ),
                claim("c-2", "table_alias", "orders", None, ClaimStatus::Candidate),
            ],
        )],
        0,
    );
    // The compact header counts the unconfirmed claim and points at the review
    // queue — the same next step the learn path names.
    assert!(
        text.contains("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
        "{text}"
    );
    // The candidate's line carries the explicit unconfirmed mark; the
    // confirmed claim's line does not.
    assert!(
        text.contains("table_alias  orders  candidate  (unconfirmed)"),
        "{text}"
    );
    assert!(
        text.contains("default_time_column  created_at  col:created_at  confirmed\n"),
        "{text}"
    );
}

/// The `/queue` pointer rides only the unconfirmed count. A
/// recall that supplied only confirmed claims must not nag, and the pre-existing
/// silence rule stands — a `Ran`-and-found-nothing recall renders nothing at
/// all (spec §4), so there is no pointer when there is nothing to point at.
#[test]
fn the_queue_pointer_rides_only_the_unconfirmed_count() {
    let confirmed_only = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                "c-1",
                "table_alias",
                "orders",
                None,
                ClaimStatus::Confirmed,
            )],
        )],
        0,
    );
    assert!(
        confirmed_only.contains("1 claim") && !confirmed_only.contains("/queue"),
        "no pointer when everything is confirmed: {confirmed_only}"
    );
    // The silence rule still holds: found nothing, dropped nothing → empty.
    assert_eq!(knowledge_supplied_text(ran(false), &[], 0), "");
}

/// A non-zero dropped count is shown on the header (spec §5 / §4).
#[test]
fn a_nonzero_dropped_count_is_shown() {
    // With claims kept and some dropped.
    let text = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                "c-1",
                "table_alias",
                "orders",
                None,
                ClaimStatus::Confirmed,
            )],
        )],
        30,
    );
    assert!(text.contains("· 30 more dropped by bounds"), "{text}");

    // All claims dropped by bounds: supplied is empty but dropped > 0 — this
    // is NOT silence; the dropped count is the signal (spec §4).
    let all_dropped = knowledge_supplied_text(ran(false), &[], 30);
    assert!(
        all_dropped.contains("memory supplied · 0 claims"),
        "{all_dropped}"
    );
    assert!(
        all_dropped.contains("· 30 more dropped by bounds"),
        "{all_dropped}"
    );
}
