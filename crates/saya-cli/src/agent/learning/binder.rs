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
#[path = "binder_tests.rs"]
mod tests;
