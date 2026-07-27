use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "saya", version, about = "Database-aware AI for the terminal")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    Ask {
        prompt: Option<String>,
    },
    Query {
        #[arg(long)]
        sql: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Init,
    Doctor,
    Show,
}

#[derive(Debug, Subcommand)]
enum ConnectionCommand {
    List,
    Test { profile: String },
    Schema { profile: String },
}

fn main() {
    let cli = Cli::parse();
    if cli.command.is_none() {
        println!("Interactive SAYA is planned for a later foundation slice.");
    }
}
