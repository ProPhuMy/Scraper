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

pub async fn sync(cfg: &Config, db_path: &Path) -> anyhow::Result<()> {
    let client = portals::client()?;
    let raw = simplify::download(&client, cfg.season_year).await?;
    let jobs = simplify::filter_and_normalize(&raw);

    if jobs.is_empty() {
        anyhow::bail!("no jobs matched filter rules; refusing to sync empty list");
    }

    let mut conn = db::open(db_path)?;
    let stats = db::upsert_jobs(&mut conn, &jobs)?;
    println!(
        "synced {} jobs: {} new, {} updated",
        stats.total, stats.inserted, stats.requeued
    );
    Ok(())
}

pub async fn scan(
    cfg: &Config,
    db_path: &Path,
    limit: Option<usize>,
    no_llm: bool,
) -> anyhow::Result<()> {
    let conn = db::open(db_path)?;
    let pending = db::pending_jobs(&conn, limit)?;
    let db_conn = Arc::new(Mutex::new(conn));

    if pending.is_empty() {
        println!("nothing to scan");
    } else {
        let global_sem = Arc::new(tokio::sync::Semaphore::new(cfg.scan.concurrency));
        let mut host_sems = HashMap::new();
        for job in &pending {
            let host = host_of(&job.url);
            host_sems
                .entry(host)
                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(cfg.scan.per_host)));
        }

        let done_counter = Arc::new(AtomicUsize::new(0));
        let client = portals::client()?;
        let total = pending.len();

        let mut set = tokio::task::JoinSet::new();

        for job in pending {
            let host = host_of(&job.url);
            let host_sem = host_sems.get(&host).expect("host semaphore exists").clone();
            let global_sem = global_sem.clone();
            let db_conn = db_conn.clone();
            let client = client.clone();
            let done_counter = done_counter.clone();
            let season_year = cfg.season_year;

            set.spawn(async move {
                // host first: waiting on a busy host must not hold a global slot
                let _host_permit = host_sem.acquire_owned().await.expect("semaphore valid");
                let _global_permit = global_sem.acquire_owned().await.expect("semaphore valid");

                let fetch_res = portals::fetch(&client, &job).await;
                drop(_global_permit);
                drop(_host_permit);

                let status = match fetch_res {
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
                };

                let done = done_counter.fetch_add(1, Ordering::Relaxed) + 1;
                println!(
                    "[{done}/{total}] {:<10} {} \u{2014} {}",
                    status.as_str(),
                    job.company,
                    job.title
                );
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
                    Status::RulesDone => rules_done_count += 1,
                    Status::Unreadable => unreadable_count += 1,
                    Status::Gone => gone_count += 1,
                    Status::Error => error_count += 1,
                    _ => {}
                },
                Err(join_err) => {
                    eprintln!("task join error: {join_err:#}");
                    error_count += 1;
                }
            }
        }

        println!(
            "Scan complete: {rules_done_count} rules_done, {unreadable_count} unreadable, {gone_count} gone, {error_count} error"
        );
    }

    // LLM phase
    if no_llm {
        println!("LLM skipped (--no-llm)");
        return Ok(());
    }

    let llm_cfg = match &cfg.llm {
        Some(c) => c,
        None => {
            println!("LLM not configured; unknown facts stay \"maybe\"");
            return Ok(());
        }
    };

    let llm_client = match LlmClient::from_config(llm_cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            return Ok(());
        }
    };

    let candidates = {
        let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
        db::llm_candidates(&conn, limit)?
    };

    for (job, page, facts) in candidates {
        match llm_client
            .settle(&job.title, &page, &facts, cfg.season_year)
            .await
        {
            Ok(LlmOutcome::Facts(f)) => {
                let conn = db_conn.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = db::save_llm(&conn, job.id, &f) {
                    eprintln!("failed to save LLM facts for job {}: {e:#}", job.id);
                }
            }
            Ok(LlmOutcome::QuotaExhausted) => {
                println!("LLM quota reached; run scan again later");
                break;
            }
            Err(e) => {
                eprintln!("LLM error for job {}: {e:#}", job.id);
            }
        }
    }

    Ok(())
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
}
