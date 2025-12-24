use anyhow::Result;
use clap::{Parser, Subcommand};

mod bun;
mod client;
mod cmd;
mod components;
mod engine;
mod ocel;
mod project;
mod server;
mod tofu;
mod utils;

#[derive(Parser)]
#[command(author, version, about = "Ocel CLI", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Create a new Ocel project
    Init,
    /// Run your project in development mode
    Dev,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Dev => {
            cmd::dev()?;
        }
        Commands::Init => {
            cmd::init()?;
        }
    }

    Ok(())
}
