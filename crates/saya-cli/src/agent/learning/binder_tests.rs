//! Tests for the pure note binder over the verified catalog.

use super::*;
use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

fn table(profile_char: char, profile_name: &str, name: &str) -> VerifiedTable {
    VerifiedTable {
        profile_name: profile_name.to_owned(),
        object: DatabaseObjectRef::new(
            ProfileIdentity::parse(&format!("p-{}", profile_char.to_string().repeat(64))).unwrap(),
            "db",
            "public",
            name,
            DatabaseObjectKind::Table,
        )
        .unwrap(),
    }
}

#[test]
fn matches_whole_names_case_insensitively_and_binds_every_named_table() {
    let orders = table('a', "warehouse", "orders");
    let refunds = table('a', "warehouse", "refunds");
    let notes = bind_notes(
        &["The ORDERS table and `refunds` are separate sources.".to_owned()],
        &[orders.clone(), refunds.clone()],
        CatalogCompleteness::Complete,
    );
    assert_eq!(notes.len(), 2);
    assert!(
        notes
            .iter()
            .all(|note| note.sentence == "The ORDERS table and `refunds` are separate sources.")
    );
    assert!(notes.iter().any(|note| note.object == orders.object));
    assert!(notes.iter().any(|note| note.object == refunds.object));
    assert!(
        bind_notes(
            &["preorders are delayed.".to_owned()],
            &[orders],
            CatalogCompleteness::Complete
        )
        .is_empty()
    );
}

#[test]
fn ambiguous_unmatched_and_database_only_names_are_refused() {
    let a = table('a', "alpha", "orders");
    let b = table('b', "beta", "orders");
    assert!(
        bind_notes(
            &["The orders table is important.".to_owned()],
            &[a.clone(), b.clone()],
            CatalogCompleteness::Complete
        )
        .is_empty()
    );
    let selected = bind_notes(
        &["In beta, the orders table is delayed.".to_owned()],
        &[a, b.clone()],
        CatalogCompleteness::Complete,
    );
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].object, b.object);
    assert!(
        bind_notes(
            &["Use warehouse for totals.".to_owned()],
            &[table('a', "warehouse", "orders")],
            CatalogCompleteness::Complete
        )
        .is_empty()
    );
    assert!(
        bind_notes(
            &["customers are active.".to_owned()],
            &[table('a', "warehouse", "orders")],
            CatalogCompleteness::Complete
        )
        .is_empty()
    );
    let city = table('a', "world_1", "city");
    let dogs = table('b', "kennels", "dogs");
    let rental = table('c', "pagila", "rental");
    for sentence in [
        "a big city has residents.",
        "count dogs by breed.",
        "a rental is long.",
    ] {
        assert!(
            bind_notes(
                &[sentence.to_owned()],
                &[city.clone(), dogs.clone(), rental.clone()],
                CatalogCompleteness::Complete
            )
            .is_empty()
        );
    }
}

#[test]
fn incomplete_catalog_requires_an_explicit_profile() {
    let alpha = table('a', "alpha", "orders");
    assert!(
        bind_notes(
            &["The orders table is delayed.".into()],
            std::slice::from_ref(&alpha),
            CatalogCompleteness::Incomplete
        )
        .is_empty()
    );
    assert_eq!(
        bind_notes(
            &["In alpha, the orders table is delayed.".into()],
            &[alpha],
            CatalogCompleteness::Incomplete
        )
        .len(),
        1
    );
}

#[test]
fn profile_qualified_name_binds_in_incomplete_catalog_only_as_exact_path() {
    let table = table('a', "world_1", "city");
    let yes = bind_notes(
        &["world_1.city is the city dimension.".into()],
        std::slice::from_ref(&table),
        CatalogCompleteness::Incomplete,
    );
    assert_eq!(yes.len(), 1);
    let no = bind_notes(
        &["x.world_1.city is listed.".into()],
        &[table],
        CatalogCompleteness::Incomplete,
    );
    assert!(no.is_empty());
}

#[test]
fn more_than_two_thousand_entries_forces_incomplete_semantics() {
    let mut tables: Vec<_> = (0..MAX_VERIFIED_TABLES_INSPECTED)
        .map(|index| table('a', "alpha", &format!("t{index}")))
        .collect();
    let sentence = "The t0 table is important.".to_owned();
    assert_eq!(
        bind_notes(&[sentence], &tables, CatalogCompleteness::Complete).len(),
        1
    );
    tables.push(table('b', "beta", "t0"));
    assert!(
        bind_notes(
            &["The t0 table is important.".into()],
            &tables,
            CatalogCompleteness::Complete,
        )
        .is_empty()
    );
}

#[test]
fn keeps_distinct_sentences_for_the_same_table() {
    let orders = table('a', "warehouse", "orders");
    let notes = bind_notes(
        &[
            "The orders table is one row per sale.".into(),
            "The orders table excludes refunds.".into(),
        ],
        std::slice::from_ref(&orders),
        CatalogCompleteness::Complete,
    );
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().all(|note| note.object == orders.object));
}
