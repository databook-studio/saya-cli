//! Native deterministic diagnostic producer; compiled only by the owning tests.
use std::{env, fs, process};

const WARNING: &str = r#"{"$message_type":"diagnostic","message":"unused variable x","code":{"code":"unused_variables","explanation":null},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}],"children":[],"rendered":null}"#;
const ERROR: &str = r#"{"$message_type":"diagnostic","message":"cannot find value missing","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"tracked.txt","line_start":2,"line_end":2,"column_start":1,"column_end":2,"is_primary":true}],"children":[],"rendered":null}"#;

fn emit(record: &str, cargo: bool) {
    if cargo {
        println!(r#"{{"reason":"compiler-message","message":{record}}}"#);
    } else {
        eprintln!("{record}");
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let cargo = args.next().as_deref() == Some("cargo");
    if args.next().as_deref() == Some("malformed") {
        if cargo {
            println!("not-json");
        } else {
            eprintln!("not-json");
        }
        process::exit(23);
    }
    let source = fs::read_to_string("tracked.txt").expect("owned candidate input exists");
    if source.starts_with("empty") {
        process::exit(19);
    }
    if source.starts_with("oversized") {
        for _ in 0..1000 {
            emit(WARNING, cargo);
        }
        process::exit(31);
    }
    emit(WARNING, cargo);
    if source.contains("duplicate") {
        emit(WARNING, cargo);
    }
    if source.starts_with("changed") {
        emit(ERROR, cargo);
        process::exit(1);
    }
}
