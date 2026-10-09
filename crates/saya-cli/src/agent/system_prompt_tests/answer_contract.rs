use super::*;

/// The single largest class of benchmark failures was the right numbers in the
/// wrong presentation, so the assembled prompt must brief the model on the
/// shape of an answer. The contract is a section in its own right, pushed
/// unconditionally alongside [`WORKING_GUIDANCE`].
#[test]
fn assembled_prompt_contains_the_answer_contract() {
    let prompt = assemble_system_prompt_with_session(
        &single_registry("main"),
        MemoryMode::Off,
        false,
        &facts(&single_registry("main"), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains(ANSWER_CONTRACT),
        "the answer contract must be part of every prompt: {prompt}"
    );
}

/// The contract governs how the model writes any answer, so it is neither a
/// memory concern nor a multi-connection concern: it must appear in every
/// assembled prompt regardless of how many databases are connected or whether
/// memory is on, reachable, or off.
#[test]
fn answer_contract_present_regardless_of_connections_and_memory() {
    let cases: [(ConnectionRegistry, MemoryMode, bool); 6] = [
        (single_registry("main"), MemoryMode::Off, false),
        (single_registry("main"), MemoryMode::Assisted, true),
        (single_registry("main"), MemoryMode::Assisted, false),
        (multi_registry(), MemoryMode::Off, false),
        (multi_registry(), MemoryMode::Assisted, true),
        (multi_registry(), MemoryMode::Off, true),
    ];
    for (reg, mode, reachable) in cases {
        let prompt =
            assemble_system_prompt_with_session(&reg, mode, reachable, &facts(&reg, &None))
                .expect("a prompt for this case");
        assert!(
            prompt.contains(ANSWER_CONTRACT),
            "answer contract missing for memory {}, reachable {reachable}: {prompt}",
            mode.as_str(),
        );
    }
}

/// A measured class of benchmark failure: the agent gathers the operands of
/// the computation a question asks for and stops, leaving the division or the
/// subtraction to the reader. A question asking for a ratio or a percentage
/// wants the quotient, not the two numbers it divides. Pinning the contract's
/// compute directive here, not just in [`ANSWER_CONTRACT`], because the
/// assembled prompt is what the model actually reads.
#[test]
fn answer_contract_directs_completing_a_computation_not_returning_its_operands() {
    let prompt = assemble_system_prompt_with_session(
        &single_registry("main"),
        MemoryMode::Off,
        false,
        &facts(&single_registry("main"), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("a ratio, a percentage, or a difference"),
        "the directive must name the class of computations it covers: {prompt}"
    );
    assert!(
        prompt.contains("compute that value and answer with it"),
        "the contract must tell the model to finish the computation the question names: {prompt}"
    );
    // The directive targets the failure, not a query shape: the failures were
    // correct operands handed back with the arithmetic left undone.
    assert!(
        prompt.contains("returning the operands alone stops one step short"),
        "the directive must name the failure it fixes — stopping one step short: {prompt}"
    );
}

/// A superlative names one thing. "Which driver had the fastest lap" has a
/// single-row answer; handing back every driver ordered by lap time makes the
/// reader find the answer the model already knew. The superlative clause is
/// the same rule as the computation clause looked at from the other side: both
/// are the agent stopping short of the value asked for.
#[test]
fn answer_contract_directs_a_superlative_to_name_one_row() {
    let prompt = assemble_system_prompt_with_session(
        &single_registry("main"),
        MemoryMode::Off,
        false,
        &facts(&single_registry("main"), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("\"the fastest\", \"the highest\", \"the top one\""),
        "the directive must carry the examples the earlier contract was measured on: {prompt}"
    );
    // A superlative runs in both directions; "the fewest" keeps the rule from
    // being read as a maximum only.
    assert!(
        prompt.contains("\"the fewest\""),
        "the directive must cover a superlative in the other direction: {prompt}"
    );
    assert!(
        prompt.contains("asks which one: answer with that row"),
        "the contract must tell the model a superlative has a one-row answer: {prompt}"
    );
    // Ties are governed by the clause above: a superlative must not be read as
    // permission to drop rows to manufacture a single answer.
    assert!(
        prompt.contains("Every row tied with it is part of the answer"),
        "the one-row directive must not license dropping tied rows: {prompt}"
    );
}

#[test]
fn answer_contract_section_stays_under_documented_ceiling() {
    assert!(
        ANSWER_CONTRACT.len() <= ANSWER_CONTRACT_MAX_BYTES,
        "answer contract is {} bytes; the stated ceiling is {}",
        ANSWER_CONTRACT.len(),
        ANSWER_CONTRACT_MAX_BYTES,
    );
}
