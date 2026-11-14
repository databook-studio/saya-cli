/// `designate_answer` nominates the statement that answered the question, not
/// an exploratory probe. Two questions nominated nothing at all and one
/// nominated a probe during evaluation; the description must say plainly that a
/// probe is never the answering query.
#[test]
fn designate_answer_description_forbids_an_exploratory_probe() {
    let tools = DatabaseTools::definitions(true, false, false, false);
    let designate = tools
        .iter()
        .find(|tool| tool.name == "designate_answer")
        .expect("designate_answer must be registered when query data is allowed");
    let description = &designate.description;
    assert!(
        description.contains("never an exploratory probe"),
        "the description must plainly forbid nominating a probe: {description}"
    );
    assert!(
        description.contains("the statement that produced the answer"),
        "the description must name the answering statement: {description}"
    );
}
