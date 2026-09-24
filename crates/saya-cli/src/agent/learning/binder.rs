#![allow(dead_code)] // The runtime consumes this pure binder in the following integration packet.

use saya_types::DatabaseObjectRef;

use super::turn_table::MAX_TURN_OBJECTS;

pub(crate) const MAX_VERIFIED_TABLES_INSPECTED: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogCompleteness {
    Complete,
    Incomplete,
}

#[derive(Clone)]
pub(crate) struct VerifiedTable {
    pub(crate) profile_name: String,
    pub(crate) object: DatabaseObjectRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundUserNote {
    pub(crate) sentence: String,
    pub(crate) object: DatabaseObjectRef,
}

/// Binds only explicitly written table identifiers from the verified catalog.
pub(crate) fn bind_notes(
    sentences: &[String],
    catalog: &[VerifiedTable],
    completeness: CatalogCompleteness,
) -> Vec<BoundUserNote> {
    let truncated = catalog.len() > MAX_VERIFIED_TABLES_INSPECTED;
    let completeness = if truncated {
        CatalogCompleteness::Incomplete
    } else {
        completeness
    };
    let catalog = catalog.iter().take(MAX_VERIFIED_TABLES_INSPECTED);
    let mut notes = Vec::new();
    for sentence in sentences {
        let candidates: Vec<_> = catalog
            .clone()
            .filter(|table| explicit_table_reference(sentence, table))
            .collect();
        let mut names = Vec::<String>::new();
        for table in &candidates {
            if !names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(table.object.object()))
            {
                names.push(table.object.object().to_owned());
            }
        }
        for name in names {
            let mut matches: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|table| table.object.object().eq_ignore_ascii_case(&name))
                .collect();
            let qualified: Vec<_> = matches
                .iter()
                .copied()
                .filter(|table| qualified_reference(sentence, &table.object))
                .collect();
            if !qualified.is_empty() {
                matches = qualified;
            }
            let named_profiles: Vec<_> = matches
                .iter()
                .filter(|table| {
                    contains_identifier(sentence, &table.profile_name)
                        || profile_qualified_reference(sentence, table)
                })
                .collect();
            if !named_profiles.is_empty() {
                let profile_count = named_profiles
                    .iter()
                    .fold(Vec::<&str>::new(), |mut acc, t| {
                        if !acc
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(&t.profile_name))
                        {
                            acc.push(&t.profile_name);
                        }
                        acc
                    });
                if profile_count.len() != 1 {
                    continue;
                }
                let profile = profile_count[0];
                matches.retain(|table| table.profile_name.eq_ignore_ascii_case(profile));
            } else if completeness == CatalogCompleteness::Incomplete || matches.len() != 1 {
                continue;
            }
            if matches.len() != 1 {
                continue;
            }
            let object = matches[0].object.clone();
            if notes
                .iter()
                .any(|note: &BoundUserNote| note.object == object && note.sentence == *sentence)
            {
                continue;
            }
            notes.push(BoundUserNote {
                sentence: sentence.clone(),
                object,
            });
            if notes.len() == MAX_TURN_OBJECTS {
                return notes;
            }
        }
    }
    notes
}

fn explicit_table_reference(sentence: &str, table: &VerifiedTable) -> bool {
    let name = table.object.object();
    if [
        format!("\"{name}\""),
        format!("`{name}`"),
        format!("[{name}]"),
    ]
    .iter()
    .any(|quoted| contains_case_insensitive(sentence, quoted))
        || contains_identifier(sentence, &format!("table {name}"))
        || contains_identifier(sentence, &format!("{name} table"))
    {
        return true;
    }
    qualified_reference(sentence, &table.object) || profile_qualified_reference(sentence, table)
}

fn qualified_reference(sentence: &str, object: &DatabaseObjectRef) -> bool {
    [
        format!("{}.{}", object.schema(), object.object()),
        format!(
            "{}.{}.{}",
            object.catalog(),
            object.schema(),
            object.object()
        ),
    ]
    .iter()
    .any(|qualified| contains_identifier(sentence, qualified))
}

fn profile_qualified_reference(sentence: &str, table: &VerifiedTable) -> bool {
    contains_identifier(
        sentence,
        &format!("{}.{}", table.profile_name, table.object.object()),
    )
}

fn contains_identifier(sentence: &str, identifier: &str) -> bool {
    contains_case_insensitive(sentence, identifier)
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(&needle).any(|(start, matched)| {
        let end = start + matched.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        !before.is_some_and(identifier_char) && !after.is_some_and(identifier_char)
    })
}

fn identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '$' | '-' | '.' | '#')
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

    fn table(profile_char: char, profile_name: &str, name: &str) -> VerifiedTable {
        VerifiedTable {
            profile_name: profile_name.to_owned(),
            object: DatabaseObjectRef::new(
                ProfileIdentity::parse(&format!("p-{}", profile_char.to_string().repeat(64)))
                    .unwrap(),
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
        assert!(notes.iter().all(|note| note.sentence == "The ORDERS table and `refunds` are separate sources."));
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
}
