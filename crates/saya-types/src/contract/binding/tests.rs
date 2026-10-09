use super::*;
use crate::contract::claim_enums::Cardinality;
use crate::contract::identity::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};
use crate::schema::{Column, Database, Schema, Table};
use proptest::prelude::*;

fn make_table(name: &str, cols: &[(&str, &str)]) -> Table {
    Table {
        name: name.to_string(),
        columns: cols
            .iter()
            .map(|(cname, ctype)| Column {
                name: (*cname).to_string(),
                data_type: (*ctype).to_string(),
                nullable: true,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn make_target() -> DatabaseObjectRef {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
}

#[path = "tests/columns.rs"]
mod columns;
#[path = "tests/derivation.rs"]
mod derivation;
#[path = "tests/properties.rs"]
mod properties;
#[path = "tests/relationship_derivation.rs"]
mod relationship_derivation;
#[path = "tests/requirements.rs"]
mod requirements;
#[path = "tests/serde.rs"]
mod serde;
#[path = "tests/tree_resolution.rs"]
mod tree_resolution;
#[path = "tests/type_validation.rs"]
mod type_validation;
#[path = "tests/validity.rs"]
mod validity;
