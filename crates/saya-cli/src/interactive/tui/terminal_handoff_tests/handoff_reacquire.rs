#[test]
fn order_is_release_work_reacquire_and_result_is_returned() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        42
    });
    assert_eq!(result.unwrap(), 42, "the work's value comes back");
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}

#[test]
fn work_error_value_still_reacquires() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        Err::<i32, &str>("wizard said no")
    });
    assert_eq!(
        result.unwrap(),
        Err("wizard said no"),
        "an error value is a result, not a terminal failure"
    );
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}

#[test]
fn panic_in_work_reacquires_then_resumes() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = catch_unwind(AssertUnwindSafe(|| {
        with_terminal_released(&mut recorder, || {
            log.push("work");
            panic!("wizard blew up");
        })
    }));
    let payload = result.expect_err("a panic in the work resumes after the terminal is back");
    let message = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or_default();
    assert!(
        message.contains("wizard blew up"),
        "the original panic message: {message}"
    );
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "work", "reacquire"],
        "the panic path still reacquires"
    );
}

#[test]
fn failed_release_skips_work_and_repairs_best_effort() {
    let mut recorder = Recorder::failing_release();
    let log = recorder.log.clone();
    let mut work_ran = false;
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        work_ran = true;
        7
    });
    let error = result.expect_err("a failed release runs no work");
    assert!(error.to_string().contains("release failed"), "{error}");
    assert!(!work_ran, "work never ran after a failed release");
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the half-released terminal gets a best-effort repair"
    );
}

#[test]
fn failed_reacquire_reports_the_terminal_lost() {
    let mut recorder = Recorder::failing_reacquire();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        9
    });
    let error = result.expect_err("a failed reacquire is an error");
    let message = error.to_string();
    assert!(
        message.contains("could not be re-entered"),
        "the error says the terminal is lost: {message}"
    );
    assert!(
        message.contains("reacquire failed"),
        "the underlying cause is kept: {message}"
    );
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}
