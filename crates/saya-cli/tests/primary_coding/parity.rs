use clap::Parser;
use saya_cli::{Cli, Command, SlashCommand, parse_slash_command};
#[test]
fn cli_and_slash_share_native_coding_command_grammar() {
    for tail in [
        "show job-a",
        "cancel job-a",
        "pause job-a agent-a",
        "message job-a agent-a 'check the null case'",
        "start job-a 'change one file' --write tracked.txt --approve",
        "retain-checkout job-a checkout-a --approve",
    ] {
        let slash = parse_slash_command(&format!("/coding {tail}"))
            .unwrap()
            .unwrap();
        let command = match slash {
            SlashCommand::Coding(command) => command,
            _ => panic!("coding command"),
        };
        let words = match tail {
            "show job-a" => vec!["show", "job-a"],
            "cancel job-a" => vec!["cancel", "job-a"],
            "pause job-a agent-a" => vec!["pause", "job-a", "agent-a"],
            "message job-a agent-a 'check the null case'" => {
                vec!["message", "job-a", "agent-a", "check the null case"]
            }
            "retain-checkout job-a checkout-a --approve" => {
                vec!["retain-checkout", "job-a", "checkout-a", "--approve"]
            }
            _ => vec![
                "start",
                "job-a",
                "change one file",
                "--write",
                "tracked.txt",
                "--approve",
            ],
        };
        let cli = Cli::try_parse_from(["saya", "coding"].into_iter().chain(words)).unwrap();
        assert!(matches!(cli.command,Some(Command::Coding {command:actual,..}) if actual==command));
    }
}
