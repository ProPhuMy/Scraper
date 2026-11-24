use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::db;
use crate::extract;
use crate::llm::{LlmClient, LlmOutcome};
use crate::model::{Fetched, Status};
use crate::portals;
use crate::simplify;

pub fn host_of(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => parsed.host_str().unwrap_or("").to_ascii_lowercase(),
        Err(_) => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Synced {
        total: usize,
        inserted: usize,
        requeued: usize,
    },
    NothingToScan,
    ScanStarted {
        total: usize,
    },
    Scanned {
        done: usize,
        total: usize,
        status: Status,
        company: String,
        title: String,
    },
    ScanJoinError(String),
    ScanFinished {
        rules_done: usize,
        unreadable: usize,
        gone: usize,
        error: usize,
    },
    LlmSkippedFlag,
    LlmNotConfigured,
    LlmClientError(String),
    LlmStarted {
        total: usize,
    },
    LlmSettled {
        done: usize,
        total: usize,
        company: String,
        title: String,
        changes: Vec<(String, String)>,
    },
    LlmSaveError {
        job_id: i64,
        error: String,
    },
    LlmQuota,
    LlmError {
        job_id: i64,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecheckAction {
    Touch,
    Save(crate::model::PortalPage),
    MarkGone,
}

pub fn decide_recheck(
    old_page: Option<&crate::model::PortalPage>,
    fetched: &anyhow::Result<Fetched>,
) -> RecheckAction {
    match fetched {
        Ok(Fetched::Gone) => RecheckAction::MarkGone,
        Ok(Fetched::Page(new_page)) => {
            if let Some(old) = old_page {
                if old == new_page {
                    RecheckAction::Touch
                } else {
                    RecheckAction::Save(new_page.clone())
                }
            } else {
                RecheckAction::Save(new_page.clone())
            }
        }
        Ok(Fetched::Unreadable(_)) | Err(_) => RecheckAction::Touch,
    }
}

pub type Reporter = std::sync::Arc<dyn Fn(Progress) + Send + Sync>;

pub fn format_progress(p: &Progress) -> Option<(bool, String)> {
    match p {
        Progress::Synced {
            total,
            inserted,
            requeued,
        } => Some((
            false,
            format!("synced {total} jobs: {inserted} new, {requeued} updated"),
        )),
        Progress::NothingToScan => Some((false, "nothing to scan".to_string())),
        Progress::ScanStarted { .. } => None,
        Progress::Scanned {
            done,
            total,
            status,
            company,
            title,
        } => Some((
            false,
            format!(
                "[{done}/{total}] {:<10} {company} \u{2014} {title}",
                status.as_str()
            ),
        )),
        Progress::ScanJoinError(e) => Some((true, format!("task join error: {e}"))),
        Progress::ScanFinished {
            rules_done,
            unreadable,
            gone,
            error,
        } => Some((
            false,
            format!(
                "Scan complete: {rules_done} rules_done, {unreadable} unreadable, {gone} gone, {error} error"
            ),
        )),
        Progress::LlmSkippedFlag => Some((false, "LLM skipped (--no-llm)".to_string())),
        Progress::LlmNotConfigured => Some((
            false,
            "LLM not configured; unknown facts stay \"maybe\"".to_string(),
        )),
        Progress::LlmClientError(e) => Some((true, e.clone())),
        Progress::LlmStarted { .. } => None,
        Progress::LlmSettled {
            done,
            total,
            company,
            title,
            changes,
        } => {
            if changes.is_empty() {
                Some((
                    false,
                    format!("[LLM {done}/{total}] {company} \u{2014} {title}: no change"),
                ))
            } else {
                let changes_str = changes
                    .iter()
                    .map(|(k, v)| format!("{k} \u{2192} {v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some((
                    false,
                    format!(
                        "[LLM {done}/{total}] {company} \u{2014} {title}: {changes_str}"
                    ),
                ))
            }
        }
        Progress::LlmSaveError { job_id, error } => Some((
            true,
            format!("failed to save LLM facts for job {job_id}: {error}"),
        )),
        Progress::LlmQuota => {
            Some((false, "LLM quota reached; run scan again later".to_string()))
        }
        Progress::LlmError { job_id, error } => {
            Some((true, format!("LLM error for job {job_id}: {error}")))
        }
    }
}

pub fn print_progress(p: &Progress) {
    if let Some((is_stderr, msg)) = format_progress(p) {
        if is_stderr {
            eprintln!("{msg}");
        } else {
            println!("{msg}");
        }
    }
}

pub async fn sync_with(
    cfg: &Config,
    db_path: &Path,
    report: Reporter,
) -> anyhow::Result<db::SyncStats> {
    let client = portals::client()?;
    let raw = simplify::download(&client, cfg.season_year).await?;
    let jobs = simplify::filter_and_normalize(&raw);

    if jobs.is_empty() {
        anyhow::bail!("no jobs matched filter rules; refusing to sync empty list");
    }

    let mut conn = db::open(db_path)?;
    let stats = db::upsert_jobs(&mut conn, &jobs)?;
    report(Progress::Synced {
        total: stats.total,
        inserted: stats.inserted,
        requeued: stats.requeued,
    });
    Ok(stats)
}

pub async fn sync(cfg: &Config, db_path: &Path) -> anyhow::Result<()> {
    sync_with(cfg, db_path, Arc::new(|p| print_progress(&p))).await?;
    Ok(())
}

pub async fn scan_with(
    cfg: &Config,
    db_path: &Path,
    limit: Option<usize>,
    no_llm: bool,
    report: Reporter,
) -> anyhow::Result<()> {
    let conn = db::open(db_path)?;
    let pending = db::pending_jobs(&conn, limit)?;
    let now = db::now_unix();
    let recheck_limit = match limit {
        Some(l) => l.saturating_sub(pending.len()).min(cfg.scan.recheck_max),
        None => cfg.scan.recheck_max,
    };
    let recheck = if recheck_limit > 0 && cfg.scan.recheck_days > 0 {
        db::recheck_jobs(&conn, cfg.scan.recheck_days, recheck_limit, now)?
    } else {
        Vec::new()
    };
    let pending_ids: std::collections::HashSet<i64> = pending.iter().map(|j| j.id).collect();
    let recheck_filtered: Vec<crate::model::Job> = recheck
        .into_iter()
        .filter(|j| !pending_ids.contains(&j.id))
        .collect();

    let work_items: Vec<(crate::model::Job, bool)> = pending
        .into_iter()
        .map(|j| (j, false))
        .chain(recheck_filtered.into_iter().map(|j| (j, true)))
        .collect();

    let db_conn = Arc::new(Mutex::new(conn));

    if work_items.is_empty() {
        report(Progress::NothingToScan);
    } else {
        let total = work_items.len();
        report(Progress::ScanStarted { total });

        let global_sem = Arc::new(tokio::sync::Semaphore::new(cfg.scan.concurrency));
        let mut host_sems = HashMap::new();
        for (job, _) in &work_items {
            let host = host_of(&job.url);
            host_sems
                .entry(host)
                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(cfg.scan.per_host)));
        }

        let done_counter = Arc::new(AtomicUsize::new(0));
        let client = portals::client()?;

        let mut set = tokio::task::JoinSet::new();

        for (job, is_recheck) in work_items {
            let host = host_of(&job.url);
            let host_sem = host_sems.get(&host).expect("host semaphore exists").clone();
            let global_sem = global_sem.clone();
            let db_conn = db_conn.clone();
            let client = client.clone();
            let done_counter = done_counter.clone();
            let season_year = cfg.season_year;
            let report = report.clone();

            set.spawn(async move {
                // host first: waiting on a busy host must not hold a global slot
                let _host_permit = host_sem.acquire_owned().await.expect("semaphore valid");
                let _global_permit = global_sem.acquire_owned().await.expect("semaphore valid");

                let fetch_res = portals::fetch(&client, &job).await;
                drop(_global_permit);
                drop(_host_permit);

                let status = if !is_recheck {
                    match fetch_res {
                        Ok(Fetched::Page(p)) => {
                            let facts = extract::extract(&job.title, &p, season_year);
                            let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                            match db::save_page(&conn, job.id, &p, &facts) {
                                Ok(()) => Status::RulesDone,
                                Err(e) => {
                                    let err_str = format!("{e:#}");
                                    let _ = db::mark(&conn, job.id, Status::Error, Some(&err_str));
                                    Status::Error
                                }
                            }
                        }
                        Ok(Fetched::Gone) => {
                            let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                            match db::mark(&conn, job.id, Status::Gone, None) {
                                Ok(()) => Status::Gone,
                                Err(e) => {
                                    let err_str = format!("{e:#}");
                                    let _ = db::mark(&conn, job.id, Status::Error, Some(&err_str));
                                    Status::Error
                                }
                            }
                        }
                        Ok(Fetched::Unreadable(why)) => {
                            let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                            match db::mark(&conn, job.id, Status::Unreadable, Some(&why)) {
                                Ok(()) => Status::Unreadable,
                                Err(e) => {
                                    let err_str = format!("{e:#}");
                                    let _ = db::mark(&conn, job.id, Status::Error, Some(&err_str));
                                    Status::Error
                                }
                            }
                        }
                        Err(e) => {
                            let err_str = format!("{e:#}");
                            let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                            let _ = db::mark(&conn, job.id, Status::Error, Some(&err_str));
                            Status::Error
                        }
                    }
                } else {
                    let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                    let old_page = if matches!(fetch_res, Ok(Fetched::Page(_))) {
                        db::stored_page(&conn, job.id).unwrap_or(None)
                    } else {
                        None
                    };
                    let action = decide_recheck(old_page.as_ref(), &fetch_res);
                    match action {
                        RecheckAction::MarkGone => {
                            let _ = db::mark(&conn, job.id, Status::Gone, None);
                            Status::Gone
                        }
                        RecheckAction::Save(p) => {
                            let facts = extract::extract(&job.title, &p, season_year);
                            // Text-only changes ("posted 3 days ago", banners) must not throw away
                            // LLM answers: only re-save when the rule-extracted facts differ.
                            let same_facts = old_page
                                .as_ref()
                                .map(|old| extract::extract(&job.title, old, season_year) == facts)
                                .unwrap_or(false);
                            if same_facts {
                                let _ = db::touch_scanned(&conn, job.id, now);
                                conn.query_row("SELECT status FROM jobs WHERE id = ?1", rusqlite::params![job.id], |r| r.get::<_, String>(0))
                                    .ok()
                                    .as_deref()
                                    .map(Status::parse)
                                    .unwrap_or(Status::RulesDone)
                            } else {
                                let _ = db::save_page(&conn, job.id, &p, &facts);
                                Status::RulesDone
                            }
                        }
                        RecheckAction::Touch => {
                            let _ = db::touch_scanned(&conn, job.id, now);
                            let current_status: Option<String> = conn
                                .query_row(
                                    "SELECT status FROM jobs WHERE id = ?1",
                                    rusqlite::params![job.id],
                                    |r| r.get(0),
                                )
                                .ok();
                            current_status
                                .as_deref()
                                .map(Status::parse)
                                .unwrap_or(Status::RulesDone)
                        }
                    }
                };

                let done = done_counter.fetch_add(1, Ordering::Relaxed) + 1;
                report(Progress::Scanned {
                    done,
                    total,
                    status,
                    company: job.company,
                    title: job.title,
                });
                status
            });
        }

        let mut rules_done_count = 0usize;
        let mut unreadable_count = 0usize;
        let mut gone_count = 0usize;
        let mut error_count = 0usize;

        while let Some(res) = set.join_next().await {
            match res {
                Ok(status) => match status {
                    // A touched llm_done row keeps its status and counts in the rules_done bucket in ScanFinished
                    Status::RulesDone | Status::LlmDone => rules_done_count += 1,
                    Status::Unreadable => unreadable_count += 1,
                    Status::Gone => gone_count += 1,
                    Status::Error => error_count += 1,
                    _ => {}
                },
                Err(join_err) => {
                    let err_str = format!("{join_err:#}");
                    report(Progress::ScanJoinError(err_str));
                    error_count += 1;
                }
            }
        }

        report(Progress::ScanFinished {
            rules_done: rules_done_count,
            unreadable: unreadable_count,
            gone: gone_count,
            error: error_count,
        });
    }

    // LLM phase
    if no_llm {
        report(Progress::LlmSkippedFlag);
        return Ok(());
    }

    let llm_cfg = match &cfg.llm {
        Some(c) => c,
        None => {
            report(Progress::LlmNotConfigured);
            return Ok(());
        }
    };

    let llm_client = match LlmClient::from_config(llm_cfg) {
        Ok(c) => c,
        Err(e) => {
            report(Progress::LlmClientError(format!("{e:#}")));
            return Ok(());
        }
    };

    let candidates = {
        let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
        db::llm_candidates(&conn, limit)?
    };

    let total = candidates.len();
    report(Progress::LlmStarted { total });

    let mut done = 0;
    for (job, page, facts) in candidates {
        match llm_client
            .settle(&job.title, &page, &facts, cfg.season_year)
            .await
        {
            Ok(LlmOutcome::Facts(f)) => {
                let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = db::save_llm(&conn, job.id, &f) {
                    report(Progress::LlmSaveError {
                        job_id: job.id,
                        error: format!("{e:#}"),
                    });
                }
                done += 1;
                let changes = crate::llm::fact_changes(&facts, &f);
                report(Progress::LlmSettled {
                    done,
                    total,
                    company: job.company,
                    title: job.title,
                    changes,
                });
            }
            Ok(LlmOutcome::QuotaExhausted) => {
                report(Progress::LlmQuota);
                break;
            }
            Err(e) => {
                report(Progress::LlmError {
                    job_id: job.id,
                    error: format!("{e:#}"),
                });
                done += 1;
                report(Progress::LlmSettled {
                    done,
                    total,
                    company: job.company,
                    title: job.title,
                    changes: Vec::new(),
                });
            }
        }
    }

    Ok(())
}

pub async fn scan(
    cfg: &Config,
    db_path: &Path,
    limit: Option<usize>,
    no_llm: bool,
) -> anyhow::Result<()> {
    scan_with(cfg, db_path, limit, no_llm, Arc::new(|p| print_progress(&p))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Citizenship, Gpa, Sponsorship};

    #[test]
    fn host_of_cases() {
        assert_eq!(host_of("https://Jobs.Lever.co/a/b"), "jobs.lever.co");
        assert_eq!(host_of("garbage"), "");
    }

    #[test]
    fn extract_fixture_sevenresearch() {
        let page = crate::portals::greenhouse::parse(include_str!(
            "../tests/fixtures/gh_sevenresearch.json"
        ))
        .unwrap();
        let f = crate::extract::extract("Algorithmic Developer - Summer Intern", &page, 2027);
        assert_eq!(f.sponsorship.value, Sponsorship::Asked);
        assert_eq!(f.gpa.value, Gpa::Asked);
        assert_eq!(f.citizenship.value, Citizenship::NotMentioned);
    }

    #[test]
    fn extract_fixture_clarity_clearance() {
        let page = crate::portals::greenhouse::parse(include_str!(
            "../tests/fixtures/gh_clarity.json"
        ))
        .unwrap();
        assert_eq!(
            crate::extract::extract("Software Intern", &page, 2027)
                .citizenship
                .value,
            Citizenship::Required
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_sync_and_scan_small() {
        let dir = std::env::temp_dir().join(format!(
            "test_live_sync_scan_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let cfg = crate::config::parse(include_str!("../scraper.example.toml")).unwrap();
        sync(&cfg, &db_path).await.unwrap();
        scan(&cfg, &db_path, Some(5), true).await.unwrap();
        let conn = crate::db::open(&db_path).unwrap();
        let rows = crate::db::list_jobs(&conn).unwrap();
        let non_pending = rows
            .iter()
            .filter(|r| r.status != crate::model::Status::Pending)
            .count();
        assert!(non_pending >= 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_strings_unchanged() {
        assert_eq!(
            format_progress(&Progress::Synced {
                total: 1801,
                inserted: 3,
                requeued: 0,
            }),
            Some((false, "synced 1801 jobs: 3 new, 0 updated".into()))
        );
        assert_eq!(
            format_progress(&Progress::NothingToScan),
            Some((false, "nothing to scan".into()))
        );
        assert_eq!(
            format_progress(&Progress::ScanStarted { total: 10 }),
            None
        );
        assert_eq!(
            format_progress(&Progress::Scanned {
                done: 4,
                total: 60,
                status: Status::RulesDone,
                company: "Acme".into(),
                title: "SWE Intern".into(),
            }),
            Some((false, "[4/60] rules_done Acme \u{2014} SWE Intern".into()))
        );
        assert_eq!(
            format_progress(&Progress::ScanJoinError("boom".into())),
            Some((true, "task join error: boom".into()))
        );
        assert_eq!(
            format_progress(&Progress::ScanFinished {
                rules_done: 28,
                unreadable: 32,
                gone: 0,
                error: 0,
            }),
            Some((
                false,
                "Scan complete: 28 rules_done, 32 unreadable, 0 gone, 0 error".into()
            ))
        );
        assert_eq!(
            format_progress(&Progress::LlmSkippedFlag),
            Some((false, "LLM skipped (--no-llm)".into()))
        );
        assert_eq!(
            format_progress(&Progress::LlmNotConfigured),
            Some((
                false,
                "LLM not configured; unknown facts stay \"maybe\"".into()
            ))
        );
        assert_eq!(
            format_progress(&Progress::LlmClientError("invalid auth".into())),
            Some((true, "invalid auth".into()))
        );
        assert_eq!(
            format_progress(&Progress::LlmStarted { total: 15 }),
            None
        );
        assert_eq!(
            format_progress(&Progress::LlmSettled {
                done: 3,
                total: 5,
                company: "CornerStone Partners".into(),
                title: "AI and Automation Intern".into(),
                changes: vec![("class_years".into(), "any".into())],
            }),
            Some((
                false,
                "[LLM 3/5] CornerStone Partners \u{2014} AI and Automation Intern: class_years \u{2192} any".into()
            ))
        );
        assert_eq!(
            format_progress(&Progress::LlmSettled {
                done: 3,
                total: 5,
                company: "CornerStone Partners".into(),
                title: "AI and Automation Intern".into(),
                changes: vec![],
            }),
            Some((
                false,
                "[LLM 3/5] CornerStone Partners \u{2014} AI and Automation Intern: no change".into()
            ))
        );
        assert_eq!(
            format_progress(&Progress::LlmSaveError {
                job_id: 12,
                error: "write failed".into(),
            }),
            Some((
                true,
                "failed to save LLM facts for job 12: write failed".into()
            ))
        );
        assert_eq!(
            format_progress(&Progress::LlmQuota),
            Some((false, "LLM quota reached; run scan again later".into()))
        );
        assert_eq!(
            format_progress(&Progress::LlmError {
                job_id: 7,
                error: "boom".into(),
            }),
            Some((true, "LLM error for job 7: boom".into()))
        );
    }

    #[test]
    fn busy_timeout_set() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("test_busy_timeout_{}_{}.db", std::process::id(), nanos));
        let conn = crate::db::open(&db_path).unwrap();
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn recheck_failure_keeps_data() {
        let page = crate::model::PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let err: anyhow::Result<Fetched> = Err(anyhow::anyhow!("timeout or 403"));
        assert_eq!(decide_recheck(Some(&page), &err), RecheckAction::Touch);

        let unreadable: anyhow::Result<Fetched> = Ok(Fetched::Unreadable("blocked".into()));
        assert_eq!(decide_recheck(Some(&page), &unreadable), RecheckAction::Touch);

        assert_eq!(decide_recheck(None, &err), RecheckAction::Touch);
    }

    #[test]
    fn recheck_unchanged_keeps_llm() {
        let page = crate::model::PortalPage {
            description: "same desc".to_string(),
            questions: vec![],
        };
        let fetched: anyhow::Result<Fetched> = Ok(Fetched::Page(page.clone()));
        assert_eq!(decide_recheck(Some(&page), &fetched), RecheckAction::Touch);
    }

    #[test]
    fn recheck_changed_saves() {
        let old_page = crate::model::PortalPage {
            description: "old desc".to_string(),
            questions: vec![],
        };
        let new_page = crate::model::PortalPage {
            description: "new desc".to_string(),
            questions: vec![],
        };
        let fetched: anyhow::Result<Fetched> = Ok(Fetched::Page(new_page.clone()));
        assert_eq!(
            decide_recheck(Some(&old_page), &fetched),
            RecheckAction::Save(new_page.clone())
        );

        assert_eq!(
            decide_recheck(None, &fetched),
            RecheckAction::Save(new_page)
        );
    }

    #[test]
    fn recheck_gone_marks_gone() {
        let old_page = crate::model::PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let fetched: anyhow::Result<Fetched> = Ok(Fetched::Gone);
        assert_eq!(
            decide_recheck(Some(&old_page), &fetched),
            RecheckAction::MarkGone
        );
    }
}
