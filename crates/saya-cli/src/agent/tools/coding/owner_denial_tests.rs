//! The closed denial vocabulary and its run-scoped repeat guard: the class
//! names are stable, every class carries a fixed next step, and a later
//! denial of the same class collapses to the class, the count and the next
//! step. There is no job-level write-path ceiling, so the vocabulary is the
//! single no-checkout class.
use super::{DenialTally, OwnerWriteDenial};

#[test]
fn every_class_has_a_stable_name_and_one_next_step() {
    let classes = [(OwnerWriteDenial::NoWritePath, "no_write_path_approved")];
    for (denial, class) in classes {
        assert_eq!(denial.class(), class);
        assert!(
            !denial.next_step().is_empty(),
            "{} must name a next step",
            denial.class()
        );
        let message = denial.message();
        assert!(message.starts_with("owner write denied: "), "{message}");
        assert!(message.contains(denial.next_step()), "{message}");
    }
}

#[test]
fn a_repeat_collapses_to_the_class_count_and_the_next_step() {
    let tally = DenialTally::default();
    let denial = OwnerWriteDenial::NoWritePath;
    let mut wordings = Vec::new();
    for _ in 0..3 {
        tally.note(denial.class());
        wordings.push(tally.wording(&denial));
    }
    assert_eq!(wordings[0], denial.message());
    assert_eq!(
        wordings[1],
        "repeated denials (no_write_path_approved ×2): delegate the change to a helper, which \
         writes the working checkout."
    );
    assert_eq!(
        wordings[2],
        "repeated denials (no_write_path_approved ×3): delegate the change to a helper, which \
         writes the working checkout."
    );
}

#[test]
fn the_collapsed_repeat_carries_no_provider_text() {
    let tally = DenialTally::default();
    let denial = OwnerWriteDenial::NoWritePath;
    tally.note(denial.class());
    tally.note(denial.class());
    let wording = tally.wording(&denial);
    assert_eq!(
        wording,
        "repeated denials (no_write_path_approved ×2): delegate the change to a helper, which \
         writes the working checkout."
    );
}
