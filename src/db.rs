#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Job, NewJob, PortalPage, Facts, Status, Provider};

pub fn open(path: &Path) -> anyhow::Result<rusqlite::Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> anyhow::Result<rusqlite::Connection> {
    let conn = Connection::open_in_memory()?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS jobs (
            id INTEGER PRIMARY KEY,
            simplify_id TEXT UNIQUE NOT NULL,
            company TEXT NOT NULL,
            title TEXT NOT NULL,
            category TEXT NOT NULL,
            provider TEXT NOT NULL,
            url TEXT NOT NULL,
            locations TEXT NOT NULL,
            date_posted INTEGER NOT NULL,
            date_updated INTEGER NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'pending',
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            page TEXT,
            facts TEXT,
            scanned_at INTEGER
        );"
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStats {
    pub inserted: usize,
    pub requeued: usize,
    pub total: usize,
}

pub fn upsert_jobs(conn: &mut Connection, jobs: &[NewJob]) -> anyhow::Result<SyncStats> {
    if jobs.is_empty() {
        anyhow::bail!("refusing to sync an empty job list");
    }

    let tx = conn.transaction()?;
    tx.execute("UPDATE jobs SET active = 0", [])?;

    let mut seen = HashSet::new();
    let mut inserted = 0;
    let mut requeued = 0;
    let total = jobs.len();

    for job in jobs {
        if !seen.insert(&job.simplify_id) {
            continue;
        }

        let existing: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, date_updated FROM jobs WHERE simplify_id = ?1",
                params![&job.simplify_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let locations_json = serde_json::to_string(&job.locations)
            .context("failed to serialize job locations")?;

        match existing {
            None => {
                tx.execute(
                    "INSERT INTO jobs (
                        simplify_id, company, title, category, provider, url,
                        locations, date_posted, date_updated, active, status, attempts
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'pending', 0)",
                    params![
                        &job.simplify_id,
                        &job.company,
                        &job.title,
                        &job.category,
                        job.provider.as_str(),
                        &job.url,
                        &locations_json,
                        job.date_posted,
                        job.date_updated,
                    ],
                )?;
                inserted += 1;
            }
            Some((id, stored_date_updated)) => {
                if job.date_updated > stored_date_updated {
                    tx.execute(
                        "UPDATE jobs SET
                            company = ?1,
                            title = ?2,
                            category = ?3,
                            provider = ?4,
                            url = ?5,
                            locations = ?6,
                            date_posted = ?7,
                            date_updated = ?8,
                            status = 'pending',
                            attempts = 0,
                            last_error = NULL,
                            page = NULL,
                            facts = NULL,
                            scanned_at = NULL,
                            active = 1
                        WHERE id = ?9",
                        params![
                            &job.company,
                            &job.title,
                            &job.category,
                            job.provider.as_str(),
                            &job.url,
                            &locations_json,
                            job.date_posted,
                            job.date_updated,
                            id,
                        ],
                    )?;
                    requeued += 1;
                } else {
                    tx.execute(
                        "UPDATE jobs SET active = 1 WHERE id = ?1",
                        params![id],
                    )?;
                }
            }
        }
    }

    tx.commit()?;
    Ok(SyncStats {
        inserted,
        requeued,
        total,
    })
}

pub fn pending_jobs(conn: &Connection, limit: Option<usize>) -> anyhow::Result<Vec<Job>> {
    let sql = "SELECT id, simplify_id, company, title, url, provider FROM jobs
               WHERE active = 1 AND (status = 'pending' OR (status = 'error' AND attempts < 3))
               ORDER BY date_posted DESC LIMIT ?1";
    let limit_val = limit.map(|l| l as i64).unwrap_or(-1);
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![limit_val], |row| {
        let id: i64 = row.get(0)?;
        let simplify_id: String = row.get(1)?;
        let company: String = row.get(2)?;
        let title: String = row.get(3)?;
        let url: String = row.get(4)?;
        let provider_str: String = row.get(5)?;
        let provider = Provider::parse(&provider_str);
        Ok(Job {
            id,
            simplify_id,
            company,
            title,
            url,
            provider,
        })
    })?;

    let mut jobs = Vec::new();
    for job in rows {
        jobs.push(job?);
    }
    Ok(jobs)
}

pub fn save_page(conn: &Connection, job_id: i64, page: &PortalPage, facts: &Facts) -> anyhow::Result<()> {
    let page_json = serde_json::to_string(page).context("failed to serialize PortalPage")?;
    let facts_json = serde_json::to_string(facts).context("failed to serialize Facts")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("clock went backwards")?
        .as_secs() as i64;

    conn.execute(
        "UPDATE jobs SET status = 'rules_done', page = ?1, facts = ?2, scanned_at = ?3, last_error = NULL WHERE id = ?4",
        params![page_json, facts_json, now, job_id],
    )?;
    Ok(())
}

pub fn mark(conn: &Connection, job_id: i64, status: Status, error: Option<&str>) -> anyhow::Result<()> {
    if matches!(status, Status::Error) {
        conn.execute(
            "UPDATE jobs SET status = ?1, last_error = ?2, attempts = attempts + 1 WHERE id = ?3",
            params![status.as_str(), error, job_id],
        )?;
    } else {
        conn.execute(
            "UPDATE jobs SET status = ?1, last_error = ?2 WHERE id = ?3",
            params![status.as_str(), error, job_id],
        )?;
    }
    Ok(())
}

pub fn llm_candidates(conn: &Connection, limit: Option<usize>) -> anyhow::Result<Vec<(Job, PortalPage, Facts)>> {
    let mut stmt = conn.prepare(
        "SELECT id, simplify_id, company, title, url, provider, page, facts FROM jobs
         WHERE active = 1 AND status = 'rules_done'
         ORDER BY date_posted DESC"
    )?;

    struct RawRow {
        job: Job,
        page_str: Option<String>,
        facts_str: Option<String>,
    }

    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let simplify_id: String = row.get(1)?;
        let company: String = row.get(2)?;
        let title: String = row.get(3)?;
        let url: String = row.get(4)?;
        let provider_str: String = row.get(5)?;
        let page_str: Option<String> = row.get(6)?;
        let facts_str: Option<String> = row.get(7)?;
        let provider = Provider::parse(&provider_str);
        Ok(RawRow {
            job: Job {
                id,
                simplify_id,
                company,
                title,
                url,
                provider,
            },
            page_str,
            facts_str,
        })
    })?;

    let mut result = Vec::new();
    let max_items = limit.unwrap_or(usize::MAX);

    for r in rows {
        let raw = r?;
        let (page_json, facts_json) = match (raw.page_str, raw.facts_str) {
            (Some(p), Some(f)) => (p, f),
            _ => continue,
        };

        let page: PortalPage = serde_json::from_str(&page_json)
            .with_context(|| format!("failed to parse page JSON for job id {}", raw.job.id))?;
        let facts: Facts = serde_json::from_str(&facts_json)
            .with_context(|| format!("failed to parse facts JSON for job id {}", raw.job.id))?;

        if facts.has_unknown() {
            result.push((raw.job, page, facts));
            if result.len() >= max_items {
                break;
            }
        }
    }

    Ok(result)
}

pub fn save_llm(conn: &Connection, job_id: i64, facts: &Facts) -> anyhow::Result<()> {
    let facts_json = serde_json::to_string(facts).context("failed to serialize Facts")?;
    conn.execute(
        "UPDATE jobs SET status = 'llm_done', facts = ?1, last_error = NULL WHERE id = ?2",
        params![facts_json, job_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub job: Job,
    pub status: Status,
    pub facts: Option<Facts>,
    pub date_posted: i64,
}

pub fn list_jobs(conn: &Connection) -> anyhow::Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT id, simplify_id, company, title, url, provider, status, facts, date_posted FROM jobs
         WHERE active = 1
         ORDER BY date_posted DESC"
    )?;

    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let simplify_id: String = row.get(1)?;
        let company: String = row.get(2)?;
        let title: String = row.get(3)?;
        let url: String = row.get(4)?;
        let provider_str: String = row.get(5)?;
        let status_str: String = row.get(6)?;
        let facts_str: Option<String> = row.get(7)?;
        let date_posted: i64 = row.get(8)?;

        let provider = Provider::parse(&provider_str);
        let status = Status::parse(&status_str);

        Ok((
            Job {
                id,
                simplify_id,
                company,
                title,
                url,
                provider,
            },
            status,
            facts_str,
            date_posted,
        ))
    })?;

    let mut result = Vec::new();
    for r in rows {
        let (job, status, facts_str, date_posted) = r?;
        let facts = match facts_str {
            Some(s) => Some(
                serde_json::from_str::<Facts>(&s)
                    .with_context(|| format!("corrupt facts JSON for job id {}: {}", job.id, s))?,
            ),
            None => None,
        };
        result.push(Row {
            job,
            status,
            facts,
            date_posted,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClassYears, Gpa, Provider, Sponsorship, Citizenship, Experience};

    fn nj(id: &str, date_updated: i64) -> NewJob {
        NewJob {
            simplify_id: id.to_string(),
            company: "Acme".to_string(),
            title: "Software Engineer Intern".to_string(),
            category: "Software".to_string(),
            url: format!("https://example.com/jobs/{id}"),
            provider: Provider::Other,
            locations: vec!["San Francisco, CA".to_string()],
            date_posted: 1000,
            date_updated,
        }
    }

    #[test]
    fn upsert_inserts_then_noops() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        let stats = upsert_jobs(&mut c, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(stats.inserted, 2);
        assert_eq!(stats.requeued, 0);
        assert_eq!(stats.total, 2);

        let stats2 = upsert_jobs(&mut c, &[a, b]).unwrap();
        assert_eq!(stats2.inserted, 0);
        assert_eq!(stats2.requeued, 0);
        assert_eq!(stats2.total, 2);
    }

    #[test]
    fn upsert_requeues_on_newer_date() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a.clone()]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let facts = Facts::unknown();
        save_page(&c, 1, &page, &facts).unwrap();

        let a_newer = nj("a", 101);
        let stats = upsert_jobs(&mut c, &[a_newer]).unwrap();
        assert_eq!(stats.requeued, 1);

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(rows[0].facts, None);
    }

    #[test]
    fn upsert_older_date_keeps_scan() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a.clone()]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let facts = Facts::unknown();
        save_page(&c, 1, &page, &facts).unwrap();

        let a_same = nj("a", 100);
        let stats = upsert_jobs(&mut c, &[a_same]).unwrap();
        assert_eq!(stats.requeued, 0);

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::RulesDone);
        assert_eq!(rows[0].facts, Some(facts));
    }

    #[test]
    fn upsert_deactivates_missing() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        upsert_jobs(&mut c, &[a.clone(), b]).unwrap();

        upsert_jobs(&mut c, &[a]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job.simplify_id, "a");
    }

    #[test]
    fn upsert_reactivates_returning() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        upsert_jobs(&mut c, &[a.clone(), b.clone()]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let facts = Facts::unknown();
        save_page(&c, 2, &page, &facts).unwrap();

        upsert_jobs(&mut c, &[a.clone()]).unwrap();
        let rows1 = list_jobs(&c).unwrap();
        assert_eq!(rows1.len(), 1);

        upsert_jobs(&mut c, &[a, b]).unwrap();
        let rows2 = list_jobs(&c).unwrap();
        assert_eq!(rows2.len(), 2);
        let row_b = rows2.iter().find(|r| r.job.simplify_id == "b").unwrap();
        assert_eq!(row_b.status, Status::RulesDone);
    }

    #[test]
    fn upsert_empty_is_error() {
        let mut c = open_in_memory().unwrap();
        assert!(upsert_jobs(&mut c, &[]).is_err());
    }

    #[test]
    fn pending_excludes_done_and_maxed_errors() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        let cc = nj("c", 100);
        let d = nj("d", 100);
        upsert_jobs(&mut c, &[a, b, cc, d]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let facts = Facts::unknown();
        save_page(&c, 2, &page, &facts).unwrap();

        mark(&c, 3, Status::Error, Some("fail 1")).unwrap();
        mark(&c, 3, Status::Error, Some("fail 2")).unwrap();
        mark(&c, 3, Status::Error, Some("fail 3")).unwrap();

        mark(&c, 4, Status::Error, Some("fail 1")).unwrap();

        let pending = pending_jobs(&c, None).unwrap();
        let ids: Vec<String> = pending.into_iter().map(|j| j.simplify_id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"d".to_string()));
    }

    #[test]
    fn pending_respects_limit() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        let cc = nj("c", 100);
        upsert_jobs(&mut c, &[a, b, cc]).unwrap();

        let pending = pending_jobs(&c, Some(2)).unwrap();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn llm_candidates_only_unknown() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        upsert_jobs(&mut c, &[a, b]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };

        save_page(&c, 1, &page, &Facts::unknown()).unwrap();

        let mut known_facts = Facts::unknown();
        known_facts.class_years.value = ClassYears::Any;
        known_facts.gpa.value = Gpa::NotMentioned;
        known_facts.sponsorship.value = Sponsorship::NotMentioned;
        known_facts.citizenship.value = Citizenship::NotMentioned;
        known_facts.experience.value = Experience::NotMentioned;
        assert!(!known_facts.has_unknown());
        save_page(&c, 2, &page, &known_facts).unwrap();

        let candidates = llm_candidates(&c, None).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0.simplify_id, "a");
    }

    #[test]
    fn save_llm_sets_status() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        save_page(&c, 1, &page, &Facts::unknown()).unwrap();

        let mut new_facts = Facts::unknown();
        new_facts.class_years.value = ClassYears::Any;
        save_llm(&c, 1, &new_facts).unwrap();

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::LlmDone);
        assert_eq!(rows[0].facts, Some(new_facts));
    }

    #[test]
    fn duplicate_simplify_id_in_upsert_is_noop() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let a_dup = nj("a", 105);
        let stats = upsert_jobs(&mut c, &[a, a_dup]).unwrap();
        assert_eq!(stats.inserted, 1);
        assert_eq!(stats.requeued, 0);
        assert_eq!(stats.total, 2);

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job.simplify_id, "a");
    }

    #[test]
    fn open_and_reopen_on_disk() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("test_scraper_db_{}", nanos));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.db");

        {
            let mut conn = open(&db_path).unwrap();
            let a = nj("a", 100);
            upsert_jobs(&mut conn, &[a]).unwrap();
        }

        {
            let conn = open(&db_path).unwrap();
            let rows = list_jobs(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].job.simplify_id, "a");
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn mark_non_error_does_not_increment_attempts() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        mark(&c, 1, Status::Unreadable, Some("unreadable page")).unwrap();

        let attempts: i64 = c.query_row("SELECT attempts FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(attempts, 0);
    }
}
