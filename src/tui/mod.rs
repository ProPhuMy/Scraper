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
    // ratatui's hook restores the terminal on ANY panic; a panic on a background refresh/scan
    // thread must not tear down the live UI (those are reported as refresh errors instead).
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            prev_hook(info);
        }
    }));

    let res = run_loop(&mut terminal, &mut app, &conn, cfg, db_path);

    ratatui::restore();

    res
}

pub enum RefreshMsg {
    Progress(crate::commands::Progress),
    Finished(Result<(), String>),
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    conn: &rusqlite::Connection,
    cfg: &Config,
    db_path: &Path,
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<RefreshMsg>();

    while !app.quit {
        terminal.draw(|f| render(f, app))?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(action) = app.on_key(key) {
                        match action {
                            Action::Refresh => {
                                let tx = tx.clone();
                                let cfg_clone = cfg.clone();
                                let db_path_buf = db_path.to_path_buf();
                                std::thread::spawn(move || {
                                    let rt = match tokio::runtime::Runtime::new() {
                                        Ok(rt) => rt,
                                        Err(e) => {
                                            let _ = tx.send(RefreshMsg::Finished(Err(format!("{e:#}"))));
                                            return;
                                        }
                                    };
                                    let tx_mutex = std::sync::Mutex::new(tx.clone());
                                    let reporter: crate::commands::Reporter = std::sync::Arc::new(move |p| {
                                        if let Ok(tx) = tx_mutex.lock() {
                                            let _ = tx.send(RefreshMsg::Progress(p));
                                        }
                                    });
                                    // catch_unwind: a panic must still send Finished, or `r` stays blocked forever
                                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        rt.block_on(async {
                                            crate::commands::sync_with(&cfg_clone, &db_path_buf, reporter.clone()).await?;
                                            crate::commands::scan_with(&cfg_clone, &db_path_buf, None, false, reporter).await
                                        })
                                    }));
                                    let res = match res {
                                        Ok(r) => r.map_err(|e| format!("{e:#}")),
                                        Err(_) => Err("refresh crashed (internal error)".to_string()),
                                    };
                                    let _ = tx.send(RefreshMsg::Finished(res));
                                });
                            }
                            Action::SaveMarks(pairs) => {
                                for (job_id, marks) in pairs {
                                    if let Err(e) = db::set_marks(conn, job_id, marks) {
                                        app.status_msg = Some(format!("failed to save marks: {e}"));
                                    }
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
        }

        while let Ok(msg) = rx.try_recv() {
            match msg {
                RefreshMsg::Progress(p) => {
                    app.on_progress(&p);
                }
                RefreshMsg::Finished(result) => {
                    let rows = match db::list_jobs(conn) {
                        Ok(r) => r,
                        Err(e) => {
                            app.status_msg = Some(format!("reload failed: {e}"));
                            app.rows.clone()
                        }
                    };
                    app.on_refresh_finished(result, rows);
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
