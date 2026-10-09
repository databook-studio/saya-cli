#[test]
fn the_full_id_and_a_unique_prefix_resolve_to_the_one_edit() {
    let full = "ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let items = [item(full)];
    assert_eq!(resolve_id(&items, full).expect("full id"), full);
    assert_eq!(
        resolve_id(&items, "ck-aaaaaa").expect("unique 6-char prefix"),
        full,
        "the unique prefix resolves to the one edit it names"
    );
    assert_eq!(
        resolve_id(&items, "ck-aaaaaaaaaaaaaaaa").expect("longer prefix"),
        full
    );
}

#[test]
fn an_ambiguous_prefix_names_the_problem() {
    let items = [
        item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"),
    ];
    let error = resolve_id(&items, "ck-aaaaaa").expect_err("two edits share the prefix");
    assert!(
        error.contains("matches 2 recorded edits"),
        "the ambiguity is named: {error}"
    );
}

#[test]
fn a_too_short_prefix_is_refused() {
    let items = [item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
    let error = resolve_id(&items, "ck-ab").expect_err("2 characters is too short");
    assert!(
        error.contains("too short"),
        "the short prefix is named: {error}"
    );
}

#[test]
fn an_unknown_or_malformed_id_is_refused_without_reaching_the_store() {
    let items = [item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
    let error = resolve_id(&items, "ck-ffffff").expect_err("no such edit");
    assert!(
        error.contains("matches no recorded edit"),
        "the unknown prefix is named: {error}"
    );
    let error = resolve_id(&items, "zzzzzz").expect_err("no ck- prefix");
    assert!(
        error.contains("ck-"),
        "the malformed id names the id shape: {error}"
    );
}
