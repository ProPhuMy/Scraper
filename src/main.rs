mod commands;
mod config;
mod db;
mod extract;
mod model;
mod text;
mod simplify;
mod portals;
mod verdict;
mod llm;

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
        Commands::Sync => {
            let cfg = config::load(&cli.config)?;
            commands::sync(&cfg, &cli.db).await
        }
        Commands::Scan { limit, no_llm } => {
            let cfg = config::load(&cli.config)?;
            commands::scan(&cfg, &cli.db, limit, no_llm).await
        }
        Commands::List {
            year,
            verdict,
            all,
            json,
        } => {
            let cfg = config::load(&cli.config)?;
            verdict::list(&cfg, &cli.db, year, verdict, all, json)
        }
    }
}
