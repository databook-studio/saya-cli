use super::*;
use crate::contract::claim::{CLAIM_PAYLOAD_VERSION, MAX_TEXT_CHARS};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::{DatabaseObjectKind, ProfileIdentity};

fn make_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn make_target() -> DatabaseObjectRef {
    let profile = make_profile();
    DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
}

#[test]
fn constructors_reject_empty_text() {
    assert!(ClaimPayload::table_description("").is_err());
    assert!(ClaimPayload::table_grain("", None).is_err());
    assert!(ClaimPayload::column_description("col", "").is_err());
}

#[test]
fn constructors_reject_long_text() {
    let long = "x".repeat(MAX_TEXT_CHARS + 1);
    assert!(ClaimPayload::table_description(&long).is_err());
    assert!(ClaimPayload::table_grain(&long, None).is_err());
    assert!(ClaimPayload::column_description("col", &long).is_err());
}

#[test]
fn constructors_reject_control_chars() {
    assert!(ClaimPayload::table_description("hello\nworld").is_err());
    assert!(ClaimPayload::table_grain("hello\tworld", None).is_err());
    assert!(ClaimPayload::column_description("col", "text\n").is_err());
}

#[test]
fn table_alias_rejects_empty() {
    assert!(ClaimPayload::table_alias("").is_err());
}

#[test]
fn column_role_validates_column_name() {
    assert!(ClaimPayload::column_role("", ColumnRole::Dimension, None).is_err());
    assert!(ClaimPayload::column_role("col\n", ColumnRole::Dimension, None).is_err());
}

#[test]
fn default_time_column_validates_column_name() {
    assert!(ClaimPayload::default_time_column("", None).is_err());
}

// --- The reason a directive claim carries (spec: claim-reasons). ---
//
// A directive claim (`DefaultTimeColumn`, `TableGrain`, `ColumnRole`) may
// carry the *why* alongside the *what*, so a model that would otherwise
// argue with it reads the justification. The reason is optional, free text,
// bounded and validated exactly the way `TableDescription`'s text is
// (`validate_text`: non-empty, ≤ MAX_TEXT_CHARS, no control characters).

#[test]
fn default_time_column_carries_an_optional_reason() {
    // No reason: the directive claims a user made before this change, and
    // the common case where only the value is stated.
    let none = ClaimPayload::default_time_column("return_date", None).unwrap();
    assert!(matches!(
        none,
        ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
    ));
    let with = ClaimPayload::default_time_column(
        "return_date",
        Some("a rental only counts once it comes back"),
    )
    .unwrap();
    assert!(matches!(
        with,
        ClaimPayload::DefaultTimeColumn { reason: Some(r), .. } if r == "a rental only counts once it comes back"
    ));
}

#[test]
fn table_grain_carries_an_optional_reason() {
    let none = ClaimPayload::table_grain("one row per order", None).unwrap();
    assert!(matches!(
        none,
        ClaimPayload::TableGrain { reason: None, .. }
    ));
    let with =
        ClaimPayload::table_grain("one row per order", Some("orders ship separately")).unwrap();
    assert!(matches!(
        with,
        ClaimPayload::TableGrain { reason: Some(r), .. } if r == "orders ship separately"
    ));
}

#[test]
fn column_role_carries_an_optional_reason() {
    let none = ClaimPayload::column_role("amount", ColumnRole::Measure, None).unwrap();
    assert!(matches!(
        none,
        ClaimPayload::ColumnRole { reason: None, .. }
    ));
    let with = ClaimPayload::column_role(
        "amount",
        ColumnRole::Measure,
        Some("money the customer paid"),
    )
    .unwrap();
    assert!(matches!(
        with,
        ClaimPayload::ColumnRole { reason: Some(r), .. } if r == "money the customer paid"
    ));
}

#[test]
fn reason_is_validated_like_description_text() {
    // Too long and control characters are rejected by the same
    // `validate_text` bound `TableDescription` uses — a reason is a
    // claim-shaped free-text field, not an unbounded annotation. An empty
    // or whitespace-only reason carries no information, so it collapses to
    // `None` (no reason) rather than erroring: the field is optional, and a
    // user who typed only spaces meant none.
    let long = "x".repeat(MAX_TEXT_CHARS + 1);
    assert!(ClaimPayload::default_time_column("c", Some(&long)).is_err());
    assert!(ClaimPayload::table_grain("g", Some(&long)).is_err());
    assert!(ClaimPayload::column_role("c", ColumnRole::Measure, Some(&long)).is_err());

    assert!(ClaimPayload::default_time_column("c", Some("line\nbreak")).is_err());
    assert!(ClaimPayload::table_grain("g", Some("tab\there")).is_err());
    assert!(ClaimPayload::column_role("c", ColumnRole::Measure, Some("ctl\0char")).is_err());

    // An empty or whitespace-only reason collapses to `None` — no error, no
    // reason stored. The value is still constructed.
    let empty = ClaimPayload::default_time_column("c", Some("   ")).unwrap();
    assert!(matches!(
        empty,
        ClaimPayload::DefaultTimeColumn { reason: None, .. }
    ));
    let blank = ClaimPayload::table_grain("g", Some("")).unwrap();
    assert!(matches!(
        blank,
        ClaimPayload::TableGrain { reason: None, .. }
    ));
}

/// An old payload written before the reason field exists must decode back
/// as `reason: None`, not fail. A decode failure on the read path would
/// take out a user's entire memory, and a missing reason is the state of
/// every claim written before this change. The JSON is hand-written, not
/// generated by the current serializer — a round-trip proves nothing about
/// the old format.
#[test]
fn old_payload_without_reason_decodes_as_none() {
    let old_default_time = r#"{"kind":"default_time_column","column":"return_date"}"#;
    let p: ClaimPayload = serde_json::from_str(old_default_time).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
    ));

    let old_grain = r#"{"kind":"table_grain","description":"one row per order"}"#;
    let p: ClaimPayload = serde_json::from_str(old_grain).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::TableGrain { ref description, ref reason } if description == "one row per order" && reason.is_none()
    ));

    let old_role = r#"{"kind":"column_role","column":"amount","role":"measure"}"#;
    let p: ClaimPayload = serde_json::from_str(old_role).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::ColumnRole { ref column, role: ColumnRole::Measure, ref reason } if column == "amount" && reason.is_none()
    ));
}

#[test]
fn relationship_rejects_empty_column_lists() {
    let target = make_target();
    assert!(
        ClaimPayload::relationship(
            target.clone(),
            vec![],
            vec!["b".into()],
            Cardinality::OneToOne
        )
        .is_err()
    );
    assert!(
        ClaimPayload::relationship(
            target.clone(),
            vec!["a".into()],
            vec![],
            Cardinality::OneToOne
        )
        .is_err()
    );
    assert!(
        ClaimPayload::relationship(
            target,
            vec!["a".into()],
            vec!["b".into()],
            Cardinality::OneToOne
        )
        .is_ok()
    );
}

#[test]
fn relationship_rejects_mismatched_lengths() {
    let target = make_target();
    let result = ClaimPayload::relationship(
        target,
        vec!["a".into(), "b".into()],
        vec!["x".into()],
        Cardinality::ManyToOne,
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), ContractError::ColumnCountMismatch);
}

#[test]
fn relationship_rejects_too_many_columns() {
    let target = make_target();
    let cols: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    let result = ClaimPayload::relationship(target, cols.clone(), cols, Cardinality::OneToMany);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), ContractError::TooManyColumns);
}

#[test]
fn relationship_validates_column_names() {
    let target = make_target();
    assert!(
        ClaimPayload::relationship(
            target.clone(),
            vec!["".into()],
            vec!["x".into()],
            Cardinality::OneToOne,
        )
        .is_err()
    );
    assert!(
        ClaimPayload::relationship(
            target,
            vec!["x".into()],
            vec!["".into()],
            Cardinality::OneToOne,
        )
        .is_err()
    );
}

#[test]
fn join_rule_rejects_empty_or_bad_condition_and_target() {
    // The condition is the free-text body; an empty one carries no fact.
    assert!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "",
            None
        )
        .is_err()
    );
    // A control character in the condition is rejected, not scrubbed.
    assert!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "join\nhere",
            None
        )
        .is_err()
    );
    // An empty target names no relation.
    assert!(
        ClaimPayload::join_rule(
            "",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None
        )
        .is_err()
    );
}

#[test]
fn join_rule_pairs_columns_and_bounds_the_count() {
    // Mismatched lengths break the positional pairing a join key list needs.
    assert_eq!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["a".into(), "b".into()],
            vec!["x".into()],
            "cond",
            None
        )
        .unwrap_err(),
        ContractError::ColumnCountMismatch
    );
    // Both lists may be empty — a predicate-only join with no declared keys.
    assert!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec![],
            vec![],
            "orders joins customers where customers.is_active",
            None
        )
        .is_ok()
    );
    // A bad column name is rejected.
    assert!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["".into()],
            vec!["id".into()],
            "cond",
            None
        )
        .is_err()
    );
    // The column count is bounded.
    let many: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    assert_eq!(
        ClaimPayload::join_rule("catalog.public.customers", many.clone(), many, "cond", None)
            .unwrap_err(),
        ContractError::TooManyColumns
    );
}

#[test]
fn join_rule_carries_an_optional_reason() {
    let none = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    assert!(matches!(none, ClaimPayload::JoinRule { reason: None, .. }));
    let with = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id",
        Some("only active customers count"),
    )
    .unwrap();
    assert!(matches!(
        with,
        ClaimPayload::JoinRule { reason: Some(r), .. } if r == "only active customers count"
    ));
}

#[test]
fn metric_definition_rejects_empty_name_and_definition() {
    assert!(ClaimPayload::metric_definition("", "SUM(amount)", vec![], None).is_err());
    assert!(ClaimPayload::metric_definition("mrr", "", vec![], None).is_err());
    assert!(ClaimPayload::metric_definition("mrr", "SUM(amount)\n", vec![], None).is_err());
}

#[test]
fn metric_definition_bounds_and_validates_columns() {
    assert!(ClaimPayload::metric_definition("mrr", "SUM(amount)", vec!["".into()], None).is_err());
    let many: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    assert_eq!(
        ClaimPayload::metric_definition("mrr", "SUM(amount)", many, None).unwrap_err(),
        ContractError::TooManyColumns
    );
    // An empty column list is allowed: a metric with no column references
    // binds to the table existing.
    assert!(ClaimPayload::metric_definition("count", "COUNT(*)", vec![], None).is_ok());
}

#[test]
fn metric_definition_carries_an_optional_reason() {
    let none = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into()],
        None,
    )
    .unwrap();
    assert!(matches!(
        none,
        ClaimPayload::MetricDefinition { reason: None, .. }
    ));
    let with = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into()],
        Some("recurring revenue only"),
    )
    .unwrap();
    assert!(matches!(
        with,
        ClaimPayload::MetricDefinition { reason: Some(r), .. } if r == "recurring revenue only"
    ));
}

/// A join rule or metric written before the reason field exists must decode
/// with `reason: None`, not fail — a decode failure on the read path would
/// take out a user's entire memory. The JSON is hand-written to prove the
/// old shape loads, not just that the current serializer round-trips.
#[test]
fn old_join_rule_and_metric_without_reason_decode_as_none() {
    let old_join = r#"{"kind":"join_rule","target":"catalog.public.customers","local_columns":["customer_id"],"target_columns":["id"],"condition":"orders.customer_id = customers.id"}"#;
    let p: ClaimPayload = serde_json::from_str(old_join).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::JoinRule { ref target, ref condition, ref reason, .. }
            if target == "catalog.public.customers"
                && condition == "orders.customer_id = customers.id"
                && reason.is_none()
    ));

    let old_metric = r#"{"kind":"metric_definition","name":"mrr","definition":"SUM(subscription_amount) WHERE status = 'active'","columns":["subscription_amount","status"]}"#;
    let p: ClaimPayload = serde_json::from_str(old_metric).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::MetricDefinition { ref name, ref definition, ref columns, ref reason }
            if name == "mrr"
                && definition == "SUM(subscription_amount) WHERE status = 'active'"
                && columns.as_slice() == ["subscription_amount", "status"]
                && reason.is_none()
    ));
}

#[test]
fn join_rule_and_metric_serde_round_trip() {
    let join = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id and customers.is_active",
        Some("only active customers count"),
    )
    .unwrap();
    let json = serde_json::to_string(&join).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(join, back);

    let metric = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into(), "status".into()],
        Some("recurring revenue only"),
    )
    .unwrap();
    let json = serde_json::to_string(&metric).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, back);
}

#[test]
fn blanked_empties_join_rule_condition_but_keeps_keys_and_target() {
    let join = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id and customers.is_active",
        Some("only active customers count"),
    )
    .unwrap();
    let blanked = join.blanked();
    assert!(matches!(
        blanked,
        ClaimPayload::JoinRule {
            ref target,
            ref local_columns,
            ref target_columns,
            ref condition,
            ref reason,
        } if target == "catalog.public.customers"
            && local_columns.as_slice() == ["customer_id"]
            && target_columns.as_slice() == ["id"]
            && condition.is_empty()
            && reason.is_none()
    ));
    // The blanked payload round-trips through serde so a later read decodes.
    let json = serde_json::to_string(&blanked).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(back, blanked);
}

#[test]
fn blanked_empties_metric_definition_but_keeps_name_and_columns() {
    let metric = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into(), "status".into()],
        Some("recurring revenue only"),
    )
    .unwrap();
    let blanked = metric.blanked();
    assert!(matches!(
        blanked,
        ClaimPayload::MetricDefinition {
            ref name,
            ref definition,
            ref columns,
            ref reason,
        } if name == "mrr"
            && definition.is_empty()
            && columns.as_slice() == ["subscription_amount", "status"]
            && reason.is_none()
    ));
    let json = serde_json::to_string(&blanked).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(back, blanked);
}

#[test]
fn kind_discriminator() {
    let target = make_target();
    assert_eq!(
        ClaimPayload::table_description("desc").unwrap().kind(),
        "table_description"
    );
    assert_eq!(
        ClaimPayload::table_alias("t").unwrap().kind(),
        "table_alias"
    );
    assert_eq!(
        ClaimPayload::table_grain("g", None).unwrap().kind(),
        "table_grain"
    );
    assert_eq!(
        ClaimPayload::column_description("c", "d").unwrap().kind(),
        "column_description"
    );
    assert_eq!(
        ClaimPayload::column_role("c", ColumnRole::Identifier, None)
            .unwrap()
            .kind(),
        "column_role"
    );
    assert_eq!(
        ClaimPayload::default_time_column("c", None).unwrap().kind(),
        "default_time_column"
    );
    assert_eq!(
        ClaimPayload::relationship(
            target,
            vec!["a".into()],
            vec!["b".into()],
            Cardinality::OneToMany
        )
        .unwrap()
        .kind(),
        "relationship"
    );
    assert_eq!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None
        )
        .unwrap()
        .kind(),
        "join_rule"
    );
    assert_eq!(
        ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            None
        )
        .unwrap()
        .kind(),
        "metric_definition"
    );
}

#[test]
fn referenced_columns() {
    let target = make_target();
    assert!(
        ClaimPayload::table_description("desc")
            .unwrap()
            .referenced_columns()
            .is_empty()
    );
    assert!(
        ClaimPayload::table_alias("t")
            .unwrap()
            .referenced_columns()
            .is_empty()
    );
    assert!(
        ClaimPayload::table_grain("g", None)
            .unwrap()
            .referenced_columns()
            .is_empty()
    );
    assert_eq!(
        ClaimPayload::column_description("c", "d")
            .unwrap()
            .referenced_columns(),
        vec!["c"]
    );
    assert_eq!(
        ClaimPayload::column_role("c", ColumnRole::Measure, None)
            .unwrap()
            .referenced_columns(),
        vec!["c"]
    );
    assert_eq!(
        ClaimPayload::default_time_column("c", None)
            .unwrap()
            .referenced_columns(),
        vec!["c"]
    );
    assert_eq!(
        ClaimPayload::relationship(
            target,
            vec!["local_a".into(), "local_b".into()],
            vec!["tgt_a".into(), "tgt_b".into()],
            Cardinality::ManyToMany,
        )
        .unwrap()
        .referenced_columns(),
        vec!["local_a", "local_b"]
    );
    // A join rule depends on its local join keys — the columns on the
    // table the claim is stored against — never the target's columns.
    assert_eq!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into(), "tenant_id".into()],
            vec!["id".into(), "tenant".into()],
            "orders.customer_id = customers.id",
            None
        )
        .unwrap()
        .referenced_columns(),
        vec!["customer_id", "tenant_id"]
    );
    // A predicate-only join with no equi-join keys depends on nothing the
    // local table's columns can name, so its referenced columns are empty.
    assert!(
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec![],
            vec![],
            "orders joins customers where customers.is_active",
            None
        )
        .unwrap()
        .referenced_columns()
        .is_empty()
    );
    assert_eq!(
        ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            None
        )
        .unwrap()
        .referenced_columns(),
        vec!["subscription_amount", "status"]
    );
}

#[test]
fn serde_round_trip() {
    let payload = ClaimPayload::table_description("hello world").unwrap();
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(payload, deserialized);
}

#[test]
fn blanked_empties_free_text_but_keeps_structural_identity() {
    // The secret-bearing free-text fields are cleared to empty.
    let desc = ClaimPayload::table_description("a customers table").unwrap();
    assert_eq!(
        desc.blanked(),
        ClaimPayload::TableDescription {
            text: String::new()
        }
    );
    let alias = ClaimPayload::table_alias("customers").unwrap();
    assert_eq!(
        alias.blanked(),
        ClaimPayload::TableAlias {
            alias: String::new()
        }
    );
    let grain = ClaimPayload::table_grain("one row per order", None).unwrap();
    assert_eq!(
        grain.blanked(),
        ClaimPayload::TableGrain {
            description: String::new(),
            reason: None,
        }
    );
    // A column-scoped description keeps its column (the slot's structural
    // column, not free text) but drops the descriptive text.
    let col_desc = ClaimPayload::column_description("created_at", "when the row was made").unwrap();
    assert_eq!(
        col_desc.blanked(),
        ClaimPayload::ColumnDescription {
            column: "created_at".into(),
            text: String::new(),
        }
    );
    // A column role keeps the column and the closed-enum role — neither is
    // user free text, and both are reconstructable from the slot.
    let col_role = ClaimPayload::column_role("created_at", ColumnRole::Timestamp, None).unwrap();
    assert_eq!(
        col_role.blanked(),
        ClaimPayload::ColumnRole {
            column: "created_at".into(),
            role: ColumnRole::Timestamp,
            reason: None,
        }
    );
    let default_time = ClaimPayload::default_time_column("created_at", None).unwrap();
    assert_eq!(
        default_time.blanked(),
        ClaimPayload::DefaultTimeColumn {
            column: "created_at".into(),
            reason: None,
        }
    );
    // A directive claim's reason is user-supplied free text — a secret-bearing
    // channel like `description`/`alias` — so `forget` blanks it too. A
    // forgotten tombstone must not leak the justification a user wrote.
    let reasoned = ClaimPayload::default_time_column(
        "return_date",
        Some("a rental only counts once it comes back"),
    )
    .unwrap();
    assert!(matches!(
        reasoned.blanked(),
        ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
    ));
    let grain_reason = ClaimPayload::table_grain("one row per order", Some("why")).unwrap();
    assert!(matches!(
        grain_reason.blanked(),
        ClaimPayload::TableGrain { ref reason, .. } if reason.is_none()
    ));
    let role_reason =
        ClaimPayload::column_role("amount", ColumnRole::Measure, Some("why")).unwrap();
    assert!(matches!(
        role_reason.blanked(),
        ClaimPayload::ColumnRole { ref reason, .. } if reason.is_none()
    ));
}

#[test]
fn blanked_is_the_same_variant_and_serializes_so_reads_do_not_break() {
    // A forgotten row's `value_json` must still decode as a `ClaimPayload` or
    // every read of its object fails. `blanked` keeps the variant, so the
    // serialised form round-trips through serde.
    for payload in [
        ClaimPayload::table_description("d").unwrap(),
        ClaimPayload::table_alias("a").unwrap(),
        ClaimPayload::table_grain("g", None).unwrap(),
        ClaimPayload::column_description("c", "d").unwrap(),
        ClaimPayload::column_role("c", ColumnRole::Measure, None).unwrap(),
        ClaimPayload::default_time_column("c", None).unwrap(),
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None,
        )
        .unwrap(),
        ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into()],
            None,
        )
        .unwrap(),
    ] {
        let blanked = payload.blanked();
        assert_eq!(blanked.kind(), payload.kind(), "blanked keeps the variant");
        let json = serde_json::to_string(&blanked).unwrap();
        let back: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, blanked, "blanked payload round-trips through serde");
    }
}

fn table_with(cols: &[(&str, &str, bool)]) -> crate::Table {
    crate::Table {
        name: "t".into(),
        columns: cols
            .iter()
            .map(|(name, ty, nullable)| crate::Column {
                name: (*name).into(),
                data_type: (*ty).into(),
                nullable: *nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

#[test]
fn snapshots_resolve_type_and_nullability_from_the_live_table() {
    let table = table_with(&[("user_id", "bigint", false), ("amount", "numeric", true)]);
    let payload = ClaimPayload::column_description("user_id", "the user id").unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }]
    );
}

#[test]
fn snapshot_skips_a_referenced_column_absent_from_the_table() {
    // A claim cannot snapshot what does not exist; the caller decides what
    // that means. The helper returns only the columns it could resolve.
    let table = table_with(&[("id", "bigint", false)]);
    let payload = ClaimPayload::column_description("missing", "gone").unwrap();
    assert!(payload.referenced_column_snapshots(&table).is_empty());
}

#[test]
fn snapshots_match_case_insensitively_like_fingerprinting_does() {
    // Schema discovery and fingerprint comparison are case-insensitive on
    // object and column names; a snapshot must resolve the same way or a
    // retyped column would read as "absent" instead of "changed".
    let table = table_with(&[("User_Id", "bigint", false)]);
    let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }]
    );
}

#[test]
fn relationship_snapshots_all_local_columns() {
    let table = table_with(&[
        ("local_a", "int", false),
        ("local_b", "text", true),
        ("unrelated", "int", false),
    ]);
    let target = make_target();
    let payload = ClaimPayload::relationship(
        target,
        vec!["local_a".into(), "local_b".into()],
        vec!["x".into(), "y".into()],
        Cardinality::OneToMany,
    )
    .unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![
            ReferencedColumn {
                name: "local_a".into(),
                data_type: "int".into(),
                nullable: false,
            },
            ReferencedColumn {
                name: "local_b".into(),
                data_type: "text".into(),
                nullable: true,
            },
        ]
    );
}

#[test]
fn table_level_claims_snapshot_nothing() {
    let table = table_with(&[("id", "bigint", false)]);
    assert!(
        ClaimPayload::table_description("desc")
            .unwrap()
            .referenced_column_snapshots(&table)
            .is_empty()
    );
    assert!(
        ClaimPayload::table_alias("a")
            .unwrap()
            .referenced_column_snapshots(&table)
            .is_empty()
    );
}

#[test]
fn name_only_snapshots_record_names_without_claiming_a_type() {
    // A no-schema caller records the column names a drift rule needs, but
    // with an empty type so the reconciler treats them as unknown.
    let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
    assert_eq!(
        payload.referenced_column_name_snapshots(),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: String::new(),
            nullable: false,
        }]
    );
    assert!(
        ClaimPayload::table_description("desc")
            .unwrap()
            .referenced_column_name_snapshots()
            .is_empty()
    );
}

#[test]
fn referenced_column_serde_round_trips() {
    let col = ReferencedColumn {
        name: "amount".into(),
        data_type: "numeric".into(),
        nullable: true,
    };
    let json = serde_json::to_string(&col).unwrap();
    let back: ReferencedColumn = serde_json::from_str(&json).unwrap();
    assert_eq!(col, back);
}

#[test]
fn claim_payload_version_is_three() {
    // claim-reasons adds an optional `reason` to the directive variants.
    // The field is `#[serde(default)]`, so a version-2 row decodes with
    // `reason: None`; no migration is owed. A claim proposed before this
    // change carries version 2 (or 1) and must still decode.
    assert_eq!(CLAIM_PAYLOAD_VERSION, 3);
}
