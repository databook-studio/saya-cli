#[tokio::test]
async fn remember_column_kind_translates_to_headless_command() {
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders column-description amount order total",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnDescription,
            value: "order total".into(),
            column: Some("amount".into()),
            reason: None,
            profile: None,
        }
    );

    // column-role: the role is the value, the column precedes it.
    let cmd =
        match parse_slash_command("/remember analytics.public.orders column-role amount measure") {
            Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
            other => panic!("expected Contracts, got {other:?}"),
        };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnRole,
            value: "measure".into(),
            column: Some("amount".into()),
            reason: None,
            profile: None,
        }
    );

    // time-column: a table kind (no column); the value is the column name.
    let cmd = match parse_slash_command("/remember analytics.public.orders time-column created_at")
    {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::TimeColumn,
            value: "created_at".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );

    // A value with spaces survives because everything after the kind (and the
    // column, for column kinds) is the value — the whole point of the shape.
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders description orders fact table",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Description,
            value: "orders fact table".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );
}

/// A `/remember` with a `because <reason…>` clause translates to the same
/// `ContractsCommand::Remember` a headless `--reason` produces, so the slash
/// and headless paths agree on the reason (spec: claim-reasons, Open Question 1).
#[tokio::test]
async fn remember_because_clause_translates_to_headless_reason() {
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders time-column return_date because a rental only counts once it comes back",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::TimeColumn,
            value: "return_date".into(),
            column: None,
            reason: Some("a rental only counts once it comes back".into()),
            profile: None,
        }
    );

    // A column-role with a `because` clause: the column and role come first,
    // then the value, then the reason.
    let cmd = match parse_slash_command(
        "/remember analytics.public.orders column-role amount measure because money the customer paid",
    ) {
        Ok(Some(SlashCommand::Contracts(cmd))) => cmd,
        other => panic!("expected Contracts, got {other:?}"),
    };
    assert_eq!(
        cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::ColumnRole,
            value: "measure".into(),
            column: Some("amount".into()),
            reason: Some("money the customer paid".into()),
            profile: None,
        }
    );
}

// ---------------------------------------------------------------------------
// 11. /queue and `saya contracts queue` return the same claim ids in the same
//     order. Phase 3d parity: the slash adapter only translates `/queue` into
//     the same `ContractsCommand::Queue` the headless parser produces and hands
//     it to the shared dispatcher — no second queue read, no second DTO mapping.
// ---------------------------------------------------------------------------
