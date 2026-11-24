mod commands;
mod config;
mod db;
mod extract;
mod model;
mod text;
mod simplify;
mod portals;
mod verdict;
mod views;
mod llm;
mod tui;

use std::path::PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "scraper", about = "Internship eligibility fetcher CLI")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Init,
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
        #[arg(long)]
        starred: bool,
        #[arg(long)]
        applied: bool,
        #[arg(long)]
        new: bool,
    },
    Browse,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Commands::Init = cli.command {
        let home_dir = config::home_config_dir().ok_or_else(|| {
            anyhow::anyhow!("could not determine home config directory (APPDATA or HOME not set)")
        })?;
        let report = config::init(&home_dir)?;
        let abs_config = std::path::absolute(&report.config_path).unwrap_or(report.config_path);
        let abs_env = std::path::absolute(&report.env_path).unwrap_or(report.env_path);

        if report.config_created {
            println!("created {}", abs_config.display());
        } else {
            println!("config already exists: {}", abs_config.display());
        }

        if report.env_created {
            println!("created {}", abs_env.display());
        } else {
            println!(".env already exists: {}", abs_env.display());
        }

        println!("  edit your profile:   {}", abs_config.display());
        println!("  optional LLM key:    uncomment LLM_API_KEY in {}", abs_env.display());
        println!("  then run:            scraper sync && scraper scan && scraper browse");
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let home_dir = config::home_config_dir();
    let config_path =
        config::resolve_config_path(cli.config.as_deref(), &cwd, home_dir.as_deref())?;
    let db_path = config::resolve_db_path(cli.db.as_deref(), &config_path);

    if let Some(config_dir) = config_path.parent() {
        let env_path = config_dir.join(".env");
        dotenvy::from_path(&env_path).ok();
    }

    let cfg = config::load(&config_path)?;

    match cli.command {
        Commands::Init => unreachable!(),
        Commands::Sync => {
            commands::sync(&cfg, &db_path).await
        }
        Commands::Scan { limit, no_llm } => {
            commands::scan(&cfg, &db_path, limit, no_llm).await
        }
        Commands::List {
            year,
            verdict,
            all,
            json,
            starred,
            applied,
            new,
        } => {
            verdict::list(
                &cfg,
                &db_path,
                year,
                verdict,
                all,
                json,
                starred,
                applied,
                new,
            )
        }
        Commands::Browse => {
            tui::run(&cfg, &db_path)
        }
    }
}
