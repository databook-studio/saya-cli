use super::*;

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
