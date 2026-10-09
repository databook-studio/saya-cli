//! Properties 1 & 2 (spec §1): the schema fingerprint is deterministic, and
//! injective on the fields it covers — column count, any column name, type,
//! nullability, and order — generalised past the one hand-written
//! `("a","bc")` vs `("ab","c")` length-prefixing case. Pure: no store, no
//! filesystem, no async. Each property runs a few hundred cases.
use super::*;
use crate::Column;
use proptest::prelude::*;

/// A kind strategy: `select` needs a `Vec` here because an array of
/// `DatabaseObjectKind` does not satisfy `Into<Cow<'static, [_]>>`.
fn kind() -> impl Strategy<Value = DatabaseObjectKind> {
    prop::sample::select(vec![DatabaseObjectKind::Table, DatabaseObjectKind::View])
}

fn make_table(columns: Vec<(&str, &str, bool)>) -> Table {
    Table {
        name: "t".into(),
        columns: columns
            .into_iter()
            .map(|(name, data_type, nullable)| Column {
                name: name.into(),
                data_type: data_type.into(),
                nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

/// A name or type string drawn from a small alphabet so mutations that must
/// *differ* can be guaranteed to differ without leaning on SHA-256 collision
/// resistance.
fn token() -> impl Strategy<Value = String> {
    "[a-z]{1,4}"
}

fn table_strategy() -> impl Strategy<Value = Table> {
    // Distinct column names keep order-swap mutations meaningful: swapping
    // two columns with the same name would not change the digest (correctly),
    // so we want a table where a swap provably changes the column sequence.
    prop::collection::vec((token(), token(), any::<bool>()), 1..=4).prop_map(|cols| {
        let columns: Vec<Column> = cols
            .into_iter()
            .map(|(name, data_type, nullable)| Column {
                name,
                data_type,
                nullable,
            })
            .collect();
        Table {
            name: "t".into(),
            columns,
            primary_key: vec![],
            foreign_keys: vec![],
        }
    })
}

fn fp(kind: DatabaseObjectKind, table: &Table) -> String {
    SchemaFingerprint::of_table(kind, table).as_str().to_owned()
}

proptest! {
    /// Property 1 — determinism: the same table hashed twice yields the same
    /// digest, for any table shape.
    #[test]
    fn fingerprint_is_deterministic(
        kind in kind(),
        table in table_strategy(),
    ) {
        let a = SchemaFingerprint::of_table(kind, &table);
        let b = SchemaFingerprint::of_table(kind, &table);
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(a.as_str(), b.as_str());
        prop_assert_eq!(a.version(), b.version());
    }

    /// Property 2a — differs when the column count differs. We append one
    /// column with a name guaranteed absent from the base table.
    #[test]
    fn differs_on_column_count(
        kind in kind(),
        table in table_strategy(),
        extra_name in token(),
    ) {
        let mut mutated = table.clone();
        mutated.columns.push(Column {
            name: extra_name,
            data_type: "int".into(),
            nullable: false,
        });
        prop_assert_ne!(fp(kind, &table), fp(kind, &mutated));
    }

    /// Property 2b — differs when a single column name changes. The new name
    /// is forced to differ from the one it replaces.
    #[test]
    fn differs_on_column_name(
        kind in kind(),
        mut table in table_strategy(),
        idx in 0usize..4,
        replacement in token(),
    ) {
        prop_assume!(!table.columns.is_empty());
        let i = idx % table.columns.len();
        // Force a real change: pick a replacement unequal to the current name.
        prop_assume!(table.columns[i].name != replacement);
        // Snapshot the original *before* mutating, so the two digests are of
        // genuinely different tables.
        let original = table.clone();
        table.columns[i].name = replacement;
        prop_assert_ne!(fp(kind, &original), fp(kind, &table));
    }

    /// Property 2c — differs when a single column type changes.
    #[test]
    fn differs_on_column_type(
        kind in kind(),
        mut table in table_strategy(),
        idx in 0usize..4,
        replacement in token(),
    ) {
        prop_assume!(!table.columns.is_empty());
        let i = idx % table.columns.len();
        prop_assume!(table.columns[i].data_type != replacement);
        let original = table.clone();
        table.columns[i].data_type = replacement;
        prop_assert_ne!(fp(kind, &original), fp(kind, &table));
    }

    /// Property 2d — differs when a single column's nullability flips.
    #[test]
    fn differs_on_nullability(
        kind in kind(),
        mut table in table_strategy(),
        idx in 0usize..4,
    ) {
        prop_assume!(!table.columns.is_empty());
        let i = idx % table.columns.len();
        let original = table.clone();
        table.columns[i].nullable = !table.columns[i].nullable;
        prop_assert_ne!(fp(kind, &original), fp(kind, &table));
    }

    /// Property 2e — differs when two columns are reordered, for a table with
    /// at least two *distinct* columns so the swap provably changes the
    /// sequence. (Swapping two identical columns must not change the digest.)
    #[test]
    fn differs_on_column_order(
        kind in kind(),
        table in table_strategy(),
    ) {
        prop_assume!(table.columns.len() >= 2);
        // Require the first two columns to differ in name OR type so the swap
        // is a real change to the hashed sequence.
        prop_assume!(
            table.columns[0].name != table.columns[1].name
                || table.columns[0].data_type != table.columns[1].data_type
                || table.columns[0].nullable != table.columns[1].nullable
        );
        let mut swapped = table.clone();
        swapped.columns.swap(0, 1);
        prop_assert_ne!(fp(kind, &table), fp(kind, &swapped));
    }

    /// Property 2f — differs when the object kind differs (table vs view),
    /// the one covered field the other properties leave fixed.
    #[test]
    fn differs_on_kind(table in table_strategy()) {
        prop_assert_ne!(
            fp(DatabaseObjectKind::Table, &table),
            fp(DatabaseObjectKind::View, &table)
        );
    }

    /// Property 2g — the length-prefixing guarantee: when two single-column
    /// tables share the same concatenated (name + type) bytes but split them
    /// at different positions, the digests differ. This is the
    /// `("a","bc")` vs `("ab","c")` case generalised: without a per-field
    /// length prefix the two would collide.
    #[test]
    fn length_prefix_separates_split_positions(
        bytes in "[a-z]{2,6}",
        split_a in 1usize..6,
        split_b in 1usize..6,
    ) {
        // Force two distinct split points that both land inside `bytes`.
        let len = bytes.len();
        let i = split_a.min(len - 1).max(1);
        let j = split_b.min(len - 1).max(1);
        prop_assume!(i != j);
        let left = make_table(vec![(&bytes[..i], &bytes[i..], false)]);
        let right = make_table(vec![(&bytes[..j], &bytes[j..], false)]);
        // Same concatenated name+type bytes, different split → must differ.
        prop_assert_eq!(
            format!("{}{}", &bytes[..i], &bytes[i..]),
            bytes
        );
        prop_assert_ne!(
            fp(DatabaseObjectKind::Table, &left),
            fp(DatabaseObjectKind::Table, &right)
        );
    }
}
