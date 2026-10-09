#[test]
fn a_budget_below_the_floor_drops_the_block() {
    let items = vec![item("keep it short", 1, 1)];
    let result = render_preferences(items, super::super::sources::MIN_BLOCK_BODY_BYTES - 1);
    assert!(result.blocks.is_empty());
    assert_eq!(result.included, 0);
    assert_eq!(result.omitted, 0);
}

#[test]
fn the_first_item_that_does_not_fit_stops_the_list() {
    let long = "x".repeat(400);
    let items = vec![item(&long, 1, 1), item(&long, 2, 2), item(&long, 3, 3)];
    let one = render_preferences(vec![item(&long, 1, 1)], usize::MAX);
    let two = render_preferences(vec![item(&long, 1, 1), item(&long, 2, 2)], usize::MAX);
    let fits_one = cost_of(&one.blocks[0]);
    assert!(
        cost_of(&two.blocks[0]) > fits_one,
        "two whole items cost more"
    );
    // Room for exactly one whole item plus its omission note — never two.
    let remaining = fits_one + 128;
    let result = render_preferences(items, remaining);
    assert_eq!(result.included, 1);
    assert_eq!(result.omitted, 2);
    assert_eq!(result.blocks.len(), 1);
    assert!(result.blocks[0].truncated);
    assert!(
        result.blocks[0]
            .body
            .ends_with("… 2 more preference(s) not included (budget)"),
        "the omission count is visible: {}",
        result.blocks[0].body
    );
    assert!(
        cost_of(&result.blocks[0]) <= remaining,
        "the block fits what it claims"
    );
}

#[test]
fn the_aggregate_cap_bounds_the_block() {
    let items: Vec<ProjectPreference> = (0..20)
        .map(|index| {
            item(
                &format!("guidance {index} ").repeat(80),
                index as i64,
                index,
            )
        })
        .collect();
    let result = render_preferences(items, usize::MAX);
    assert_eq!(result.blocks.len(), 1);
    assert!(result.blocks[0].truncated, "not everything fits the cap");
    assert!(
        cost_of(&result.blocks[0]) <= MAX_PREFERENCE_REQUEST_BYTES,
        "the cap bounds the block regardless of the caller's remaining"
    );
}

#[test]
fn the_call_site_budget_honours_the_reserve_fraction() {
    // Like the instruction branch: the fraction reserve minus what the turn
    // already used, never above the preference ceiling.
    let budget = 8 * 1024;
    let used = 2 * 1024;
    assert_eq!(
        remaining_request_bytes(used, budget),
        (budget * 3 / 4).saturating_sub(used)
    );
    assert_eq!(
        remaining_request_bytes(usize::MAX / 2, budget),
        0,
        "an oversized turn leaves nothing"
    );
    let big = 64 * 1024;
    assert_eq!(
        remaining_request_bytes(0, big),
        MAX_PREFERENCE_REQUEST_BYTES,
        "the ceiling caps even a huge budget"
    );
}
