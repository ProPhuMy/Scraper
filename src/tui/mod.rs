pub mod app;
pub mod ui;

pub use app::{Action, App};
pub use ui::render;

use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;
use anyhow::Context;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use crate::config::Config;
use crate::db;

pub fn run(cfg: &Config, db_path: &Path) -> anyhow::Result<()> {
    if !std::io::stdout().is_terminal() {
        anyhow::bail!("browse needs an interactive terminal; use `scraper list` instead");
    }

    let conn = db::open(db_path).context("failed to open database")?;
    let rows = db::list_jobs(&conn).context("failed to list jobs from database")?;
    let mut app = App::new(rows, cfg);

    let mut terminal = ratatui::init();

    let res = run_loop(&mut terminal, &mut app, &conn);

    ratatui::restore();

    res
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    conn: &rusqlite::Connection,
) -> anyhow::Result<()> {
    while !app.quit {
        terminal.draw(|f| render(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if let Some(action) = app.on_key(key) {
                match action {
                    Action::SaveMarks { job_id, marks } => {
                        if let Err(e) = db::set_marks(conn, job_id, marks) {
                            app.status_msg = Some(format!("failed to save marks: {e}"));
                        }
                    }
                    Action::OpenUrl(url) => {
                        if let Err(e) = open_url(&url) {
                            app.status_msg = Some(format!("failed to open URL: {e}"));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // not `cmd /C start`: cmd treats `&` in query strings as a command separator
        Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}
