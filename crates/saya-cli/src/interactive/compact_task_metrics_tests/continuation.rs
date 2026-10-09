/// End-to-end: a compacting run saves a typed continuation on the state, with
/// the provider's summary as the generated narrative and the persisted turn
/// count as the source boundary.
#[tokio::test]
async fn a_compacting_run_saves_a_continuation_with_the_summary_as_narrative() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(4);
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, None).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed,
        "the compaction succeeds: {}",
        result.message
    );
    let Some(StoredContinuation::Current(record)) = &state.continuation else {
        panic!(
            "the compaction saved a typed continuation: {:?}",
            state.continuation
        );
    };
    let narrative = record
        .narrative
        .as_ref()
        .expect("the clean summary becomes the narrative");
    assert_eq!(
        narrative.text, "done.",
        "the narrative is the provider's summary"
    );
    assert_eq!(narrative.source, NarrativeSource::Summarizer);
    assert_eq!(
        record.source.through_turn, 4,
        "the boundary is the persisted turn count"
    );
    record.validate().expect("the record validates");
}
