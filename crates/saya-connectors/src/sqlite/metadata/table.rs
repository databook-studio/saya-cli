use std::collections::{HashMap, HashSet};

use saya_types::{Column, ForeignKey, Table};

pub(super) struct RawColumn {
    pub(super) name: String,
    pub(super) declared_type: String,
    pub(super) notnull: i64,
    pub(super) pk: i64,
}

pub(super) fn build_table(
    name: String,
    raw_columns: Vec<RawColumn>,
    wr_map: &HashMap<String, bool>,
    pk_index_set: &HashSet<String>,
    foreign_keys: Vec<ForeignKey>,
) -> Table {
    let without_rowid = wr_map.get(&name).copied().unwrap_or(false);
    let has_pk_index = pk_index_set.contains(&name);
    let pk_column_count = raw_columns.iter().filter(|c| c.pk > 0).count();

    // The pragma reports primary-key membership as a 1-based ordinal on each
    // column, so sorting by it preserves a composite key's column order —
    // the order a writer needs to match when joining on the whole key.
    let mut primary_key: Vec<(i64, String)> = raw_columns
        .iter()
        .filter(|c| c.pk > 0)
        .map(|c| (c.pk, c.name.clone()))
        .collect();
    primary_key.sort_by_key(|(ord, _)| *ord);
    let primary_key: Vec<String> = primary_key.into_iter().map(|(_, name)| name).collect();

    let columns = raw_columns
        .into_iter()
        .map(|col| {
            let rowid_alias = !without_rowid
                && pk_column_count == 1
                && col.pk == 1
                && col.declared_type.eq_ignore_ascii_case("INTEGER")
                && !has_pk_index;
            let non_null = col.notnull != 0 || (without_rowid && col.pk >= 1) || rowid_alias;
            Column {
                name: col.name,
                data_type: col.declared_type,
                nullable: !non_null,
            }
        })
        .collect();

    Table {
        name,
        columns,
        primary_key,
        foreign_keys,
    }
}
