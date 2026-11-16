use super::*;

/// The wallet refuses the claim that would overrun, and never overruns
/// under concurrent claims.
#[test]
fn claims_stop_at_the_limit_even_concurrently() {
    let budget = DownloadBudget::new(100);
    assert!(budget.claim(60));
    assert!(budget.claim(40));
    assert!(!budget.claim(1), "the next byte trips the budget");
    assert_eq!(budget.consumed(), 100);

    let shared = DownloadBudget::new(10_000);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let wallet = shared.clone();
            std::thread::spawn(move || {
                let mut claimed = 0u64;
                while wallet.claim(7) {
                    claimed += 7;
                }
                claimed
            })
        })
        .collect();
    let total: u64 = handles
        .into_iter()
        .map(|handle| handle.join().expect("join"))
        .sum();
    assert_eq!(total, shared.consumed(), "claims are atomic");
    assert!(total <= 10_000, "no overrun under concurrency");
}

/// A refused claim sets the trip latch — the event the sink reads. This
/// is the latch, not a threshold: the run consuming exactly its budget
/// (the last claim succeeding) must NOT be tripped.
#[test]
fn an_exact_fill_trips_nothing_but_a_refusal_trips() {
    let exact = DownloadBudget::new(100);
    assert!(exact.claim(60));
    assert!(exact.claim(40), "the last byte claims successfully");
    assert_eq!(exact.consumed(), 100);
    assert!(
        !exact.tripped(),
        "consuming exactly the budget with nothing refused is not a trip: \
         the sink must not pause this run"
    );

    let refused = DownloadBudget::new(100);
    assert!(refused.claim(97));
    assert!(
        !refused.claim(7),
        "the chunk does not fit in the 3 bytes of headroom"
    );
    assert_eq!(
        refused.consumed(),
        97,
        "a refusal can leave unclaimable headroom — consumed sits BELOW the \
         limit at the moment of the trip, which is exactly why the latch \
         exists and a consumed>=limit threshold would miss it"
    );
    assert!(refused.tripped(), "the refusal is the recorded event");

    // The latch rides the clone: a clone of the tripped wallet is
    // tripped. The latch records, it does not gate — a claim that fits
    // the remaining headroom still succeeds (the resume-with-headroom
    // path), and the pause is the sink's decision on the next tick.
    let clone = refused.clone();
    assert!(clone.tripped());
    assert!(
        clone.claim(3),
        "a claim fitting the headroom still succeeds"
    );
    assert_eq!(clone.consumed(), 100);
    assert!(!clone.claim(1), "past the limit the claims refuse again");
}

/// The arithmetic-overflow arm is a refusal like any other: it trips the
/// latch. A claim that cannot even be added must not read as
/// "under the limit".
#[test]
fn the_overflow_arm_trips_the_latch() {
    let budget = DownloadBudget::new(100);
    assert!(budget.claim(50));
    assert!(
        !budget.claim(u64::MAX),
        "a claim that cannot even be added is refused, never wrapped"
    );
    assert!(
        budget.tripped(),
        "the overflow arm is a refusal like any other: it trips the latch"
    );
    assert_eq!(budget.consumed(), 50, "the overflow wrote nothing");
    assert!(
        !budget.claim(51),
        "the wallet's arithmetic still governs claims"
    );
}

/// The carry arms the wallet with the spend the run's journal already
/// records — the resume path. A wallet carried to 60 of its 100 refuses
/// the claim that would overrun the *run's* budget, though a fresh
/// wallet would have accepted it: the budget binds the run, not the
/// invocation.
#[test]
fn the_carry_arms_the_wallet_with_the_spend_the_record_holds() {
    let budget = DownloadBudget::new(100);
    budget.carry(60);
    assert_eq!(budget.consumed(), 60);
    assert!(budget.claim(40), "the headroom the record leaves is real");
    assert_eq!(budget.consumed(), 100);
    assert!(
        !budget.claim(1),
        "the run's budget is spent, not a fresh one"
    );
    assert_eq!(budget.consumed(), 100, "the refused claim wrote nothing");
}

/// The carry records a level, never a refusal: a wallet carried exactly
/// to its limit is not tripped — the exact-fill posture, the twin of the
/// fresh-wallet test above. The next refused claim trips the latch the
/// ordinary way; a figure the record holds must not stand in for a
/// refusal that never happened, which is the threshold comparison the
/// latch exists to prevent.
#[test]
fn the_carry_never_synthesizes_a_refusal() {
    let exact = DownloadBudget::new(100);
    exact.carry(100);
    assert_eq!(exact.consumed(), 100);
    assert!(
        !exact.tripped(),
        "a carried level is not a refusal — no claim was ever refused"
    );
    assert!(!exact.claim(1), "past the limit the claims refuse");
    assert!(exact.tripped(), "the refusal is the recorded event");

    // Headroom carries the same way: a wallet carried below its limit
    // still claims what fits — the resume-with-headroom path the latch
    // doc names — and the first refusal after the carry trips it.
    let headroom = DownloadBudget::new(100);
    headroom.carry(97);
    assert!(!headroom.tripped());
    assert!(headroom.claim(3));
    assert!(
        !headroom.tripped(),
        "the last byte claimed, nothing refused"
    );
    assert!(!headroom.claim(1), "the next claim refuses");
    assert!(headroom.tripped());
}

/// A wallet carried past its limit — the resume that declares a smaller
/// budget than the record's spend — refuses every claim, and the first
/// one trips the latch: the record binds, the wallet never runs
/// overdrawn.
#[test]
fn a_wallet_carried_past_its_limit_refuses_every_claim() {
    let overdrawn = DownloadBudget::new(100);
    overdrawn.carry(150);
    assert_eq!(overdrawn.consumed(), 150);
    assert!(!overdrawn.tripped(), "the carry itself refuses nothing");
    assert!(
        !overdrawn.claim(1),
        "a wallet carried past its limit refuses its first claim"
    );
    assert!(overdrawn.tripped());
    assert!(!overdrawn.claim(0), "every later claim refuses too");
}

/// The seeding figure is the highest level the journal holds — levels
/// are monotone, so the highest is the spend the record carries — and
/// every other event folds in as nothing. A journal that holds no
/// download record carries nothing, never a fabricated zero.
#[test]
fn carried_from_journal_takes_the_highest_recorded_level() {
    let carried = DownloadBudget::carried_from_journal(&[
        RunEvent::RunStarted,
        RunEvent::DownloadedBytes { bytes: 60 },
        RunEvent::Usage {
            endpoint: "orchestrator".into(),
            tokens: Some(5),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
        RunEvent::DownloadedBytes { bytes: 97 },
        RunEvent::Paused {
            reason: saya_types::PauseReason::BudgetExhausted,
        },
        RunEvent::DownloadedBytes { bytes: 40 },
    ]);
    assert_eq!(
        carried, 97,
        "the highest recorded level is the spend, never a lower one"
    );
    assert_eq!(
        DownloadBudget::carried_from_journal(&[RunEvent::RunStarted]),
        0,
        "a journal that holds no download record carries nothing"
    );
}
