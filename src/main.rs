mod config;
mod db;
mod model;
mod text;
mod simplify;

use std::path::PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "scraper", about = "Internship eligibility fetcher CLI")]
pub struct Cli {
    #[arg(long, global = true, default_value = "scraper.toml")]
    pub config: PathBuf,

    #[arg(long, global = true, default_value = "scraper.db")]
    pub db: PathBuf,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Sync,
    Scan {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        no_llm: bool,
    },
    List {
        #[arg(long)]
        year: Option<u8>,
        #[arg(long)]
        verdict: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sync => anyhow::bail!("not implemented yet"),
        Commands::Scan { .. } => anyhow::bail!("not implemented yet"),
        Commands::List { .. } => anyhow::bail!("not implemented yet"),
    }
}
