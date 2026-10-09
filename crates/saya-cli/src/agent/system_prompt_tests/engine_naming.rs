use super::*;

/// SQLite has no catalog and no schema, so a three-part name is a syntax error
/// there. Telling the model to write one regardless costs a rejected query and
/// a wasted round trip on nearly every question before it corrects itself.
#[test]
fn the_naming_rule_matches_what_the_engine_accepts() {
    let sqlite_reg = registry_with(SqlDialect::Sqlite);
    let sqlite = assemble_system_prompt_with_session(
        &sqlite_reg,
        MemoryMode::Assisted,
        true,
        &facts(&sqlite_reg, &None),
    )
    .expect("a prompt");
    assert!(
        !sqlite.contains("catalog.schema.object"),
        "SQLite cannot parse a three-part name, so the prompt must not ask for one: {sqlite}"
    );

    let postgres_reg = registry_with(SqlDialect::Postgres);
    let postgres = assemble_system_prompt_with_session(
        &postgres_reg,
        MemoryMode::Assisted,
        true,
        &facts(&postgres_reg, &None),
    )
    .expect("a prompt");
    assert!(
        postgres.contains("catalog.schema.object"),
        "PostgreSQL does accept the three-part name and should still be asked for it: {postgres}"
    );
}

/// The rule exists so a remembered fact binds to a real object, and that need
/// does not go away on an engine with fewer name parts — the prompt must still
/// ask for the fullest name the engine has.
#[test]
fn every_engine_is_still_told_to_qualify_names() {
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::Mysql,
        SqlDialect::Sqlite,
        SqlDialect::DuckDb,
        SqlDialect::Snowflake,
    ] {
        let prompt = assemble_system_prompt_with_session(
            &registry_with(dialect),
            MemoryMode::Assisted,
            true,
            &facts(&registry_with(dialect), &None),
        )
        .expect("a prompt");
        assert!(
            prompt.contains(dialect.qualified_name_form()),
            "{} must be told its own name form: {prompt}",
            dialect.as_str()
        );
    }
}

/// Writing a name the engine can parse is not a memory concern: schema
/// discovery shows the same catalog/schema/table tree whether memory is on or
/// off, so the rule that keeps SQL valid has to be present either way.
#[test]
fn the_naming_rule_is_present_with_memory_off() {
    let prompt = assemble_system_prompt_with_session(
        &registry_with(SqlDialect::Sqlite),
        MemoryMode::Off,
        false,
        &facts(&registry_with(SqlDialect::Sqlite), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains(SqlDialect::Sqlite.qualified_name_form()),
        "the SQL naming rule must survive memory being off: {prompt}"
    );
    assert!(
        !prompt.contains("durable knowledge"),
        "memory is off, so the memory briefing must stay absent: {prompt}"
    );
}

/// A single connection is never named by `describe_context` (it stays silent
/// for one connection), so the prompt itself must name the engine — the model
/// is otherwise never told it is writing SQLite. The prompt must also coach
/// multi-step work, refusing to repeat a failed attempt, and giving up with a
/// reason when the question cannot be answered from what the session offers.
#[test]
fn single_connection_prompt_names_engine_and_guides_giving_up() {
    let prompt = assemble_system_prompt_with_session(
        &registry_with(SqlDialect::Postgres),
        MemoryMode::Off,
        false,
        &facts(&registry_with(SqlDialect::Postgres), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("postgresql"),
        "the engine must be named for a single connection: {prompt}"
    );
    assert!(
        prompt.contains("Multi-step work is expected"),
        "the prompt must say multi-step work is expected: {prompt}"
    );
    assert!(
        prompt.contains("Do not repeat an attempt that already failed"),
        "the prompt must tell the model not to repeat a failed attempt: {prompt}"
    );
    assert!(
        prompt.contains("Giving up with a reason"),
        "the prompt must sanction giving up with a reason: {prompt}"
    );
    assert!(
        prompt.contains("stop and say so"),
        "the prompt must tell the model to stop and explain when it cannot answer: {prompt}"
    );
}

/// The give-up guidance is not a single-connection concern: with several
/// databases the model can still hit a question no connected database can
/// answer, so the coaching must be present there too.
#[test]
fn multi_connection_prompt_also_guides_giving_up() {
    let prompt = assemble_system_prompt_with_session(
        &multi_registry(),
        MemoryMode::Off,
        false,
        &facts(&multi_registry(), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("Giving up with a reason"),
        "multi-connection prompt must also coach giving up: {prompt}"
    );
}
