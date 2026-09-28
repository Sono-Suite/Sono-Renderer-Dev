use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use renderer::{compatibility, formats};

#[derive(Parser)]
#[command(name = "renderer", about = "Sonolus Watch-mode package inspector")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    InspectEngine { engine: PathBuf },
    InspectLevel { level: PathBuf },
    InspectResources { scp: PathBuf },
    Compatibility { engine: PathBuf, resources: PathBuf, level: PathBuf },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::InspectEngine { engine } => println!("{}", serde_json::to_string_pretty(&compatibility::inspect_engine(&engine)?)?),
        Command::InspectLevel { level } => println!("{}", serde_json::to_string_pretty(&formats::load_level(&level)?)?),
        Command::InspectResources { scp } => println!("{}", serde_json::to_string_pretty(&formats::inspect_scp(&scp)?)?),
        Command::Compatibility { engine, resources, level } => println!("{}", serde_json::to_string_pretty(&compatibility::analyze(&engine, &resources, &level)?)?),
    }
    Ok(())
}
