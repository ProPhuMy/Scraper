#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Job, NewJob, PortalPage, Facts, Status, Provider};

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_secs() as i64
}

pub fn open(path: &Path) -> anyhow::Result<rusqlite::Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout = 5000;")?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> anyhow::Result<rusqlite::Connection> {
    let conn = Connection::open_in_memory()?;
    init_schema(&conn)?;
    Ok(conn)
}

pub(crate) fn init_schema(conn: &Connection) -> anyhow::Result<()> {
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
            llm_facts TEXT,
            scanned_at INTEGER,
            starred INTEGER NOT NULL DEFAULT 0,
            hidden INTEGER NOT NULL DEFAULT 0,
            applied_at INTEGER,
            first_seen INTEGER,
            note TEXT
        );
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );"
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(jobs)")?;
    let columns: HashSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;

    if !columns.contains("starred") {
        conn.execute("ALTER TABLE jobs ADD COLUMN starred INTEGER NOT NULL DEFAULT 0", [])?;
    }
    if !columns.contains("hidden") {
        conn.execute("ALTER TABLE jobs ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0", [])?;
    }
    if !columns.contains("applied_at") {
        conn.execute("ALTER TABLE jobs ADD COLUMN applied_at INTEGER", [])?;
    }
    if !columns.contains("first_seen") {
        conn.execute("ALTER TABLE jobs ADD COLUMN first_seen INTEGER", [])?;
    }
    if !columns.contains("llm_facts") {
        conn.execute("ALTER TABLE jobs ADD COLUMN llm_facts TEXT", [])?;
    }
    if !columns.contains("note") {
        conn.execute("ALTER TABLE jobs ADD COLUMN note TEXT", [])?;
    }

    // Migration: split LLM facts out of `facts` into `llm_facts`
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "SELECT id, facts, llm_facts FROM jobs WHERE facts LIKE '%\"source\":\"llm\"%' OR facts LIKE '%\"source\": \"llm\"%'"
        )?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let facts_str: Option<String> = row.get(1)?;
            let llm_facts_str: Option<String> = row.get(2)?;
            Ok((id, facts_str, llm_facts_str))
        })?;

        let mut updates = Vec::new();
        for r in rows {
            let (id, facts_str, llm_facts_str) = r?;
            if let Some(f_str) = facts_str {
                if let Ok(f) = serde_json::from_str::<Facts>(&f_str) {
                    let has_llm = f.class_years.evidence.as_ref().map(|e| e.source) == Some(crate::model::Source::Llm)
                        || f.gpa.evidence.as_ref().map(|e| e.source) == Some(crate::model::Source::Llm)
                        || f.sponsorship.evidence.as_ref().map(|e| e.source) == Some(crate::model::Source::Llm)
                        || f.citizenship.evidence.as_ref().map(|e| e.source) == Some(crate::model::Source::Llm)
                        || f.experience.evidence.as_ref().map(|e| e.source) == Some(crate::model::Source::Llm);

                    if has_llm {
                        let rules_part = f.rules_part();
                        let llm_part = f.llm_part();
                        let rules_json = serde_json::to_string(&rules_part)?;
                        let write_llm = llm_facts_str.is_none();
                        let llm_json = if write_llm {
                            Some(serde_json::to_string(&llm_part)?)
                        } else {
                            None
                        };
                        updates.push((id, rules_json, llm_json));
                    }
                }
            }
        }
        drop(stmt);

        for (id, rules_json, llm_json) in updates {
            if let Some(l_json) = llm_json {
                tx.execute(
                    "UPDATE jobs SET facts = ?1, llm_facts = ?2 WHERE id = ?3 AND llm_facts IS NULL",
                    params![rules_json, l_json, id],
                )?;
            } else {
                tx.execute(
                    "UPDATE jobs SET facts = ?1 WHERE id = ?2",
                    params![rules_json, id],
                )?;
            }
        }
    }
    tx.commit()?;

    conn.execute(
        "UPDATE jobs SET status = 'pending' WHERE status = 'unreadable' AND scanned_at IS NULL",
        [],
    )?;

    let sync_count_exists: bool = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = 'sync_count'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    if !sync_count_exists {
        let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
        let initial_count = if job_count > 0 { "1" } else { "0" };
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('sync_count', ?1)",
            params![initial_count],
        )?;
    }

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

    let previous_last_sync_at: Option<i64> = tx
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_sync_at'",
            [],
            |row| {
                let s: String = row.get(0)?;
                s.parse::<i64>().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
                })
            },
        )
        .optional()?;

    let mut now = now_unix();
    if let Some(prev) = previous_last_sync_at {
        if now <= prev {
            now = prev + 1;
        }
    }

    let current_sync_count: i64 = tx
        .query_row(
            "SELECT value FROM meta WHERE key = 'sync_count'",
            [],
            |row| {
                let s: String = row.get(0)?;
                s.parse::<i64>().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
                })
            },
        )
        .optional()?
        .unwrap_or(0);

    let next_sync_count = current_sync_count + 1;

    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('sync_count', ?1)",
        params![next_sync_count.to_string()],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('last_sync_at', ?1)",
        params![now.to_string()],
    )?;

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
                        locations, date_posted, date_updated, active, status, attempts,
                        first_seen
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'pending', 0, ?10)",
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
                        now,
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
                            llm_facts = NULL,
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
        "UPDATE jobs SET status = 'rules_done', page = ?1, facts = ?2, llm_facts = NULL, scanned_at = ?3, last_error = NULL WHERE id = ?4",
        params![page_json, facts_json, now, job_id],
    )?;
    Ok(())
}

pub fn mark(conn: &Connection, job_id: i64, status: Status, error: Option<&str>) -> anyhow::Result<()> {
    match status {
        Status::Error => {
            conn.execute(
                "UPDATE jobs SET status = ?1, last_error = ?2, attempts = attempts + 1 WHERE id = ?3",
                params![status.as_str(), error, job_id],
            )?;
        }
        Status::Unreadable => {
            conn.execute(
                "UPDATE jobs SET status = ?1, last_error = ?2, scanned_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE id = ?3",
                params![status.as_str(), error, job_id],
            )?;
        }
        _ => {
            conn.execute(
                "UPDATE jobs SET status = ?1, last_error = ?2 WHERE id = ?3",
                params![status.as_str(), error, job_id],
            )?;
        }
    }
    Ok(())
}

pub fn llm_candidates(
    conn: &Connection,
    limit: Option<usize>,
    max_age_days: u32,
    now: i64,
) -> anyhow::Result<Vec<(Job, PortalPage, Facts)>> {
    let cutoff = now.saturating_sub((max_age_days as i64) * 86400);
    let mut stmt = conn.prepare(
        "SELECT id, simplify_id, company, title, url, provider, page, facts FROM jobs
         WHERE active = 1 AND status = 'rules_done' AND applied_at IS NULL AND hidden = 0
           AND (starred = 1 OR ?1 = 0 OR date_posted >= ?2)
         ORDER BY starred DESC, date_posted DESC, id DESC"
    )?;

    struct RawRow {
        job: Job,
        page_str: Option<String>,
        facts_str: Option<String>,
    }

    let rows = stmt.query_map(params![max_age_days, cutoff], |row| {
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

pub fn save_llm(conn: &Connection, job_id: i64, llm_facts: &Facts) -> anyhow::Result<()> {
    let llm_facts_json = serde_json::to_string(llm_facts).context("failed to serialize Facts")?;
    conn.execute(
        "UPDATE jobs SET status = 'llm_done', llm_facts = ?1, last_error = NULL WHERE id = ?2",
        params![llm_facts_json, job_id],
    )?;
    Ok(())
}

pub fn stored_facts(conn: &Connection, job_id: i64) -> anyhow::Result<Option<Facts>> {
    let facts_str: Option<String> = conn
        .query_row(
            "SELECT facts FROM jobs WHERE id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()?;

    match facts_str {
        Some(s) => Ok(serde_json::from_str::<Facts>(&s).ok()),
        None => Ok(None),
    }
}

pub fn recheck_jobs(
    conn: &Connection,
    recheck_days: u32,
    max: usize,
    now: i64,
) -> anyhow::Result<Vec<Job>> {
    if recheck_days == 0 || max == 0 {
        return Ok(Vec::new());
    }
    let cutoff = now - (recheck_days as i64) * 86400;
    let sql = "SELECT id, simplify_id, company, title, url, provider FROM jobs
               WHERE active = 1
                 AND status IN ('rules_done', 'llm_done', 'unreadable')
                 AND scanned_at IS NOT NULL
                 AND scanned_at < ?1
               ORDER BY (starred = 1 OR applied_at IS NOT NULL) DESC, scanned_at ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![cutoff, max as i64], |row| {
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

pub fn stored_page(conn: &Connection, job_id: i64) -> anyhow::Result<Option<PortalPage>> {
    let page_str: Option<String> = conn
        .query_row(
            "SELECT page FROM jobs WHERE id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()?;

    match page_str {
        Some(s) => {
            let page: PortalPage = serde_json::from_str(&s)
                .with_context(|| format!("failed to deserialize PortalPage for job id {job_id}"))?;
            Ok(Some(page))
        }
        None => Ok(None),
    }
}

pub fn touch_scanned(conn: &Connection, job_id: i64, now: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE jobs SET scanned_at = ?1 WHERE id = ?2",
        params![now, job_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct Marks {
    pub starred: bool,
    pub hidden: bool,
    pub applied_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub job: Job,
    pub status: Status,
    pub facts: Option<Facts>,
    pub llm_facts: Option<Facts>,
    pub date_posted: i64,
    pub active: bool,
    pub marks: Marks,
    pub is_new: bool,
    pub locations: Vec<String>,
    pub asks_location: bool,
    pub form_checked: bool,
    pub note: Option<String>,
}

pub fn set_note(conn: &Connection, job_id: i64, note: Option<&str>) -> anyhow::Result<()> {
    let clean_note: Option<String> = note
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(200).collect());
    conn.execute(
        "UPDATE jobs SET note = ?1 WHERE id = ?2",
        params![clean_note, job_id],
    )?;
    Ok(())
}

pub fn set_marks(conn: &Connection, job_id: i64, marks: Marks) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE jobs SET starred = ?1, hidden = ?2, applied_at = ?3 WHERE id = ?4",
        params![
            if marks.starred { 1 } else { 0 },
            if marks.hidden { 1 } else { 0 },
            marks.applied_at,
            job_id,
        ],
    )?;
    Ok(())
}

pub fn list_jobs(conn: &Connection) -> anyhow::Result<Vec<Row>> {
    let mut last_sync_at: Option<i64> = None;
    let mut sync_count: i64 = 0;

    let mut meta_stmt = conn.prepare("SELECT key, value FROM meta WHERE key IN ('last_sync_at', 'sync_count')")?;
    let meta_rows = meta_stmt.query_map([], |r| {
        let k: String = r.get(0)?;
        let v: String = r.get(1)?;
        Ok((k, v))
    })?;
    for row in meta_rows {
        let (k, v) = row?;
        if k == "last_sync_at" {
            last_sync_at = v.parse::<i64>().ok();
        } else if k == "sync_count" {
            sync_count = v.parse::<i64>().unwrap_or(0);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT id, simplify_id, company, title, url, provider, status, facts, date_posted,
                active, starred, hidden, applied_at, first_seen, locations, page, llm_facts,
                note,
                CASE WHEN page IS NOT NULL AND json_valid(page) THEN COALESCE(json_array_length(page, '$.questions'), 0) > 0 ELSE 0 END
         FROM jobs
         WHERE active = 1 OR starred = 1 OR hidden = 1 OR applied_at IS NOT NULL
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
        let active_int: i64 = row.get(9)?;
        let starred_int: i64 = row.get(10)?;
        let hidden_int: i64 = row.get(11)?;
        let applied_at: Option<i64> = row.get(12)?;
        let first_seen: Option<i64> = row.get(13)?;
        let locations_str: Option<String> = row.get(14)?;
        let page_str: Option<String> = row.get(15)?;
        let llm_facts_str: Option<String> = row.get(16)?;
        let note: Option<String> = row.get(17)?;
        let form_checked_int: i64 = row.get(18)?;
        let form_checked = form_checked_int != 0;

        let provider = Provider::parse(&provider_str);
        let status = Status::parse(&status_str);
        let active = active_int != 0;
        let starred = starred_int != 0;
        let hidden = hidden_int != 0;

        let marks = Marks {
            starred,
            hidden,
            applied_at,
        };

        let is_new = sync_count >= 2
            && match (first_seen, last_sync_at) {
                (Some(fs), Some(ls)) => fs == ls,
                _ => false,
            };

        let locations: Vec<String> = locations_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let asks_location = match page_str {
            Some(ref s) => match serde_json::from_str::<PortalPage>(s) {
                Ok(page) => crate::extract::asks_location_choice(&page.questions, &locations),
                Err(_) => false,
            },
            None => false,
        };

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
            active,
            marks,
            is_new,
            locations,
            asks_location,
            llm_facts_str,
            form_checked,
            note,
        ))
    })?;

    let mut result = Vec::new();
    for r in rows {
        let (job, status, facts_str, date_posted, active, marks, is_new, locations, asks_location, llm_facts_str, form_checked, note) = r?;
        let facts = match facts_str {
            Some(s) => Some(
                serde_json::from_str::<Facts>(&s)
                    .with_context(|| format!("corrupt facts JSON for job id {}: {}", job.id, s))?,
            ),
            None => None,
        };
        let llm_facts = llm_facts_str
            .as_deref()
            .and_then(|s| serde_json::from_str::<Facts>(s).ok());
        result.push(Row {
            job,
            status,
            facts,
            llm_facts,
            date_posted,
            active,
            marks,
            is_new,
            locations,
            asks_location,
            form_checked,
            note,
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

        let candidates = llm_candidates(&c, None, 0, 1000).unwrap();
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
        assert_eq!(rows[0].facts, Some(Facts::unknown()));
        assert_eq!(rows[0].llm_facts, Some(new_facts));
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

    #[test]
    fn marks_default_false() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].marks, Marks::default());
        assert!(rows[0].active);
        assert!(!rows[0].is_new);
    }

    #[test]
    fn set_marks_roundtrip() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();
        let rows = list_jobs(&c).unwrap();
        let id = rows[0].job.id;
        let marks = Marks {
            starred: true,
            hidden: false,
            applied_at: Some(123),
        };
        set_marks(&c, id, marks).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows[0].marks, marks);
    }

    #[test]
    fn requeue_keeps_marks() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();
        let rows = list_jobs(&c).unwrap();
        let id = rows[0].job.id;
        let marks = Marks {
            starred: true,
            hidden: false,
            applied_at: Some(123),
        };
        set_marks(&c, id, marks).unwrap();

        let a_newer = nj("a", 200);
        upsert_jobs(&mut c, &[a_newer]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(rows[0].marks, marks);
    }

    #[test]
    fn list_jobs_keeps_inactive_marked() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        let cc = nj("c", 100);
        upsert_jobs(&mut c, &[a, b, cc]).unwrap();
        let rows = list_jobs(&c).unwrap();
        let id_a = rows.iter().find(|r| r.job.simplify_id == "a").unwrap().job.id;
        let id_b = rows.iter().find(|r| r.job.simplify_id == "b").unwrap().job.id;

        set_marks(
            &c,
            id_a,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(123),
            },
        )
        .unwrap();
        set_marks(
            &c,
            id_b,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
        )
        .unwrap();

        let d = nj("d", 100);
        upsert_jobs(&mut c, &[d]).unwrap();

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 3);
        let ids: HashSet<String> = rows.iter().map(|r| r.job.simplify_id.clone()).collect();
        let expected_ids: HashSet<String> = ["a", "b", "d"].into_iter().map(String::from).collect();
        assert_eq!(ids, expected_ids);

        let row_a = rows.iter().find(|r| r.job.simplify_id == "a").unwrap();
        assert!(!row_a.active);
        let row_b = rows.iter().find(|r| r.job.simplify_id == "b").unwrap();
        assert!(!row_b.active);
        let row_d = rows.iter().find(|r| r.job.simplify_id == "d").unwrap();
        assert!(row_d.active);
    }

    #[test]
    fn first_sync_nothing_new() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        upsert_jobs(&mut c, &[a, b]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].is_new);
        assert!(!rows[1].is_new);
    }

    #[test]
    fn second_sync_flags_only_inserted() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_new);

        let a2 = nj("a", 100);
        let b = nj("b", 100);
        upsert_jobs(&mut c, &[a2, b]).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 2);
        let row_a = rows.iter().find(|r| r.job.simplify_id == "a").unwrap();
        let row_b = rows.iter().find(|r| r.job.simplify_id == "b").unwrap();
        assert!(!row_a.is_new);
        assert!(row_b.is_new);
    }

    #[test]
    fn migrates_v010_db() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
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
            );
            INSERT INTO jobs (
                simplify_id, company, title, category, provider, url,
                locations, date_posted, date_updated
            ) VALUES (
                'old_1', 'OldCo', 'Engineer', 'Software', 'other', 'https://example.com/old',
                '[]', 1000, 1000
            );"
        )
        .unwrap();

        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(jobs)").unwrap();
        let cols: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(cols.contains("starred"));
        assert!(cols.contains("hidden"));
        assert!(cols.contains("applied_at"));
        assert!(cols.contains("first_seen"));

        let rows = list_jobs(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job.simplify_id, "old_1");
        assert_eq!(rows[0].marks, Marks::default());
        assert!(!rows[0].is_new);

        let sync_count: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'sync_count'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sync_count, "1");
    }

    #[test]
    fn upgraded_db_first_sync_flags_new() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
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
            );
            INSERT INTO jobs (
                simplify_id, company, title, category, provider, url,
                locations, date_posted, date_updated
            ) VALUES (
                'old_1', 'OldCo', 'Engineer', 'Software', 'other', 'https://example.com/old',
                '[]', 1000, 1000
            );"
        )
        .unwrap();

        init_schema(&conn).unwrap();

        let old_job = nj("old_1", 1000);
        let new_job = nj("new_1", 1000);
        upsert_jobs(&mut conn, &[old_job, new_job]).unwrap();

        let rows = list_jobs(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        let row_old = rows.iter().find(|r| r.job.simplify_id == "old_1").unwrap();
        let row_new = rows.iter().find(|r| r.job.simplify_id == "new_1").unwrap();
        assert!(!row_old.is_new);
        assert!(row_new.is_new);
    }

    #[test]
    fn row_has_locations_and_flag() {
        let mut c = open_in_memory().unwrap();
        let mut job = nj("job_loc", 100);
        job.locations = vec!["SF".to_string(), "NYC".to_string()];
        upsert_jobs(&mut c, &[job]).unwrap();

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].locations, vec!["SF", "NYC"]);
        assert!(!rows[0].asks_location);

        let page = PortalPage {
            description: "Some job description".to_string(),
            questions: vec![crate::model::FormQuestion {
                label: "Which office would you prefer to work in?".to_string(),
                options: vec![],
            }],
        };
        let facts = Facts::unknown();
        save_page(&c, rows[0].job.id, &page, &facts).unwrap();

        let rows_after = list_jobs(&c).unwrap();
        assert_eq!(rows_after.len(), 1);
        assert_eq!(rows_after[0].locations, vec!["SF", "NYC"]);
        assert!(rows_after[0].asks_location);
    }

    #[test]
    fn recheck_selects_old_active_scanned() {
        let mut conn = open_in_memory().unwrap();
        let now = 1_000_000;
        let ten_days_ago = now - 10 * 86400;
        let one_day_ago = now - 1 * 86400;

        let j1 = nj("j1", 100);
        let j2 = nj("j2", 100);
        let j3 = nj("j3", 100);
        let j4 = nj("j4", 100);
        let j5 = nj("j5", 100);
        upsert_jobs(&mut conn, &[j1, j2, j3, j4, j5]).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1 WHERE simplify_id = 'j1'",
            params![ten_days_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1 WHERE simplify_id = 'j2'",
            params![one_day_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'pending', scanned_at = ?1, active = 1 WHERE simplify_id = 'j3'",
            params![ten_days_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 0 WHERE simplify_id = 'j4'",
            params![ten_days_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'gone', scanned_at = ?1, active = 1 WHERE simplify_id = 'j5'",
            params![ten_days_ago],
        ).unwrap();

        let jobs = recheck_jobs(&conn, 7, 100, now).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].simplify_id, "j1");
    }

    #[test]
    fn recheck_orders_marked_first_and_caps() {
        let mut conn = open_in_memory().unwrap();
        let now = 1_000_000;
        let fifteen_days_ago = now - 15 * 86400;
        let twelve_days_ago = now - 12 * 86400;
        let eight_days_ago = now - 8 * 86400;

        let j1 = nj("j1", 100);
        let j2 = nj("j2", 100);
        let j3 = nj("j3", 100);
        upsert_jobs(&mut conn, &[j1, j2, j3]).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1, starred = 0 WHERE simplify_id = 'j1'",
            params![fifteen_days_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1, starred = 0 WHERE simplify_id = 'j2'",
            params![twelve_days_ago],
        ).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1, starred = 1 WHERE simplify_id = 'j3'",
            params![eight_days_ago],
        ).unwrap();

        let jobs = recheck_jobs(&conn, 7, 2, now).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].simplify_id, "j3");
        assert_eq!(jobs[1].simplify_id, "j1");
    }

    #[test]
    fn recheck_disabled_when_zero_days() {
        let mut conn = open_in_memory().unwrap();
        let now = 1_000_000;
        let ten_days_ago = now - 10 * 86400;

        let j1 = nj("j1", 100);
        upsert_jobs(&mut conn, &[j1]).unwrap();

        conn.execute(
            "UPDATE jobs SET status = 'rules_done', scanned_at = ?1, active = 1 WHERE simplify_id = 'j1'",
            params![ten_days_ago],
        ).unwrap();

        let jobs = recheck_jobs(&conn, 0, 100, now).unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn touch_scanned_only_changes_time() {
        let mut conn = open_in_memory().unwrap();
        let j1 = nj("j1", 100);
        upsert_jobs(&mut conn, &[j1]).unwrap();

        let page = PortalPage {
            description: "original desc".to_string(),
            questions: vec![],
        };
        let facts = Facts::unknown();
        save_page(&conn, 1, &page, &facts).unwrap();

        let original_status: String = conn.query_row("SELECT status FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let original_page: String = conn.query_row("SELECT page FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let original_facts: String = conn.query_row("SELECT facts FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let original_attempts: i64 = conn.query_row("SELECT attempts FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();

        let new_time = 9_999_999;
        touch_scanned(&conn, 1, new_time).unwrap();

        let updated_time: i64 = conn.query_row("SELECT scanned_at FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(updated_time, new_time);

        let status_after: String = conn.query_row("SELECT status FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let page_after: String = conn.query_row("SELECT page FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let facts_after: String = conn.query_row("SELECT facts FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();
        let attempts_after: i64 = conn.query_row("SELECT attempts FROM jobs WHERE id = 1", [], |r| r.get(0)).unwrap();

        assert_eq!(status_after, original_status);
        assert_eq!(page_after, original_page);
        assert_eq!(facts_after, original_facts);
        assert_eq!(attempts_after, original_attempts);
    }

    #[test]
    fn test_llm_candidates_behavior() {
        let mut conn = open_in_memory().unwrap();
        let now = 1_000_000i64;
        let day = 86400i64;

        let mut j1 = nj("j_starred_old", 100);
        j1.date_posted = now - 40 * day;

        let mut j2 = nj("j_newest", 100);
        j2.date_posted = now - 5 * day;

        let mut j3 = nj("j_middle", 100);
        j3.date_posted = now - 10 * day;

        let mut j4 = nj("j_old_unstarred", 100);
        j4.date_posted = now - 40 * day;

        let mut j5 = nj("j_applied", 100);
        j5.date_posted = now - 2 * day;

        let mut j6 = nj("j_hidden", 100);
        j6.date_posted = now - 2 * day;

        let mut j7 = nj("j_known", 100);
        j7.date_posted = now - 1 * day;

        upsert_jobs(&mut conn, &[
            j1.clone(), j2.clone(), j3.clone(), j4.clone(), j5.clone(), j6.clone(), j7.clone(),
        ]).unwrap();

        let page = PortalPage {
            description: "A description".to_string(),
            questions: vec![],
        };
        let unknown_facts = Facts::unknown();
        let mut known_facts = Facts::unknown();
        known_facts.class_years.value = ClassYears::Any;
        known_facts.gpa.value = Gpa::NotMentioned;
        known_facts.sponsorship.value = Sponsorship::NotMentioned;
        known_facts.citizenship.value = Citizenship::NotMentioned;
        known_facts.experience.value = Experience::NotMentioned;
        assert!(!known_facts.has_unknown());

        for row in list_jobs(&conn).unwrap() {
            let f = if row.job.simplify_id == "j_known" {
                &known_facts
            } else {
                &unknown_facts
            };
            save_page(&conn, row.job.id, &page, f).unwrap();
        }

        conn.execute("UPDATE jobs SET starred = 1 WHERE simplify_id = 'j_starred_old'", []).unwrap();
        conn.execute("UPDATE jobs SET applied_at = ?1 WHERE simplify_id = 'j_applied'", params![now]).unwrap();
        conn.execute("UPDATE jobs SET hidden = 1 WHERE simplify_id = 'j_hidden'", []).unwrap();

        let candidates = llm_candidates(&conn, None, 30, now).unwrap();
        let ids: Vec<_> = candidates.iter().map(|c| c.0.simplify_id.as_str()).collect();
        assert_eq!(ids, vec!["j_starred_old", "j_newest", "j_middle"]);

        let candidates_no_cutoff = llm_candidates(&conn, None, 0, now).unwrap();
        let ids_no_cutoff: Vec<_> = candidates_no_cutoff.iter().map(|c| c.0.simplify_id.as_str()).collect();
        assert_eq!(ids_no_cutoff, vec!["j_starred_old", "j_newest", "j_middle", "j_old_unstarred"]);

        let candidates_limit = llm_candidates(&conn, Some(2), 30, now).unwrap();
        let ids_limit: Vec<_> = candidates_limit.iter().map(|c| c.0.simplify_id.as_str()).collect();
        assert_eq!(ids_limit, vec!["j_starred_old", "j_newest"]);
    }

    #[test]
    fn stored_page_roundtrip() {
        let mut conn = open_in_memory().unwrap();
        let j1 = nj("j1", 100);
        upsert_jobs(&mut conn, &[j1]).unwrap();

        let page = PortalPage {
            description: "Roundtrip test".to_string(),
            questions: vec![crate::model::FormQuestion {
                label: "Q1".to_string(),
                options: vec!["A".to_string()],
            }],
        };
        save_page(&conn, 1, &page, &Facts::unknown()).unwrap();

        let loaded = stored_page(&conn, 1).unwrap();
        assert_eq!(loaded, Some(page));

        let non_existent = stored_page(&conn, 999).unwrap();
        assert_eq!(non_existent, None);
    }

    #[test]
    fn save_llm_writes_llm_facts_only() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let mut r = Facts::unknown();
        r.sponsorship.value = Sponsorship::Asked;
        save_page(&c, 1, &page, &r).unwrap();

        let mut l = Facts::unknown();
        l.class_years.value = ClassYears::Grad(std::collections::BTreeSet::from([2027]));
        save_llm(&c, 1, &l).unwrap();

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].facts, Some(r));
        assert_eq!(rows[0].llm_facts, Some(l));
        assert_eq!(rows[0].status, Status::LlmDone);
    }

    #[test]
    fn save_page_clears_llm_facts() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        let page = PortalPage {
            description: "desc".to_string(),
            questions: vec![],
        };
        let mut r = Facts::unknown();
        r.sponsorship.value = Sponsorship::Asked;
        save_page(&c, 1, &page, &r).unwrap();

        let mut l = Facts::unknown();
        l.class_years.value = ClassYears::Grad(std::collections::BTreeSet::from([2027]));
        save_llm(&c, 1, &l).unwrap();

        save_page(&c, 1, &page, &r).unwrap();

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].llm_facts, None);
        assert_eq!(rows[0].status, Status::RulesDone);
    }

    #[test]
    fn migration_splits_llm_facts() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
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
                scanned_at INTEGER,
                starred INTEGER NOT NULL DEFAULT 0,
                hidden INTEGER NOT NULL DEFAULT 0,
                applied_at INTEGER,
                first_seen INTEGER
            );
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );"
        )
        .unwrap();

        let mut merged = Facts::unknown();
        merged.class_years = crate::model::Fact {
            value: ClassYears::Grad(std::collections::BTreeSet::from([2027, 2028])),
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Llm,
                snippet: "graduating 2027 or 2028".to_string(),
            }),
        };
        merged.sponsorship = crate::model::Fact {
            value: Sponsorship::Asked,
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Form,
                snippet: "Will you require sponsorship?".to_string(),
            }),
        };
        let merged_json = serde_json::to_string(&merged).unwrap();

        conn.execute(
            "INSERT INTO jobs (
                simplify_id, company, title, category, provider, url,
                locations, date_posted, date_updated, status, facts
            ) VALUES (
                'old_1', 'OldCo', 'Engineer', 'Software', 'other', 'https://example.com/old',
                '[]', 1000, 1000, 'llm_done', ?1
            )",
            params![merged_json],
        )
        .unwrap();

        init_schema(&conn).unwrap();

        let rows = list_jobs(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].facts, Some(merged.rules_part()));
        assert_eq!(rows[0].llm_facts, Some(merged.llm_part()));

        init_schema(&conn).unwrap();
        let rows2 = list_jobs(&conn).unwrap();
        assert_eq!(rows2[0].facts, Some(merged.rules_part()));
        assert_eq!(rows2[0].llm_facts, Some(merged.llm_part()));
    }

    #[test]
    fn migration_leaves_rules_rows_alone() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
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
                scanned_at INTEGER,
                starred INTEGER NOT NULL DEFAULT 0,
                hidden INTEGER NOT NULL DEFAULT 0,
                applied_at INTEGER,
                first_seen INTEGER
            );
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );"
        )
        .unwrap();

        let mut rules_facts = Facts::unknown();
        rules_facts.sponsorship = crate::model::Fact {
            value: Sponsorship::Asked,
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Form,
                snippet: "sponsorship question".to_string(),
            }),
        };
        let rules_json = serde_json::to_string(&rules_facts).unwrap();

        conn.execute(
            "INSERT INTO jobs (
                simplify_id, company, title, category, provider, url,
                locations, date_posted, date_updated, status, facts
            ) VALUES (
                'rules_row', 'RulesCo', 'Engineer', 'Software', 'other', 'https://example.com/rules',
                '[]', 1000, 1000, 'rules_done', ?1
            )",
            params![rules_json],
        )
        .unwrap();

        init_schema(&conn).unwrap();

        let rows = list_jobs(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].facts, Some(rules_facts));
        assert_eq!(rows[0].llm_facts, None);
    }

    #[test]
    fn mark_unreadable_sets_scanned_at() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        let b = nj("b", 100);
        let d = nj("d", 100);
        upsert_jobs(&mut c, &[a, b, d]).unwrap();

        let before = now_unix();
        mark(&c, 1, Status::Unreadable, Some("x")).unwrap();
        let after = now_unix();

        let (status, scanned_at, last_error, attempts): (String, Option<i64>, Option<String>, i64) = c
            .query_row(
                "SELECT status, scanned_at, last_error, attempts FROM jobs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();

        assert_eq!(status, "unreadable");
        assert!(scanned_at.is_some());
        let ts = scanned_at.unwrap();
        assert!(ts >= before && ts <= after + 1);
        assert_eq!(last_error.as_deref(), Some("x"));
        assert_eq!(attempts, 0);

        // mark Status::Gone on another job leaves scanned_at NULL
        mark(&c, 2, Status::Gone, None).unwrap();
        let (status_b, scanned_at_b, attempts_b): (String, Option<i64>, i64) = c
            .query_row(
                "SELECT status, scanned_at, attempts FROM jobs WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status_b, "gone");
        assert_eq!(scanned_at_b, None);
        assert_eq!(attempts_b, 0);

        // mark Status::Error leaves scanned_at NULL and increments attempts
        mark(&c, 3, Status::Error, Some("err")).unwrap();
        let (status_d, scanned_at_d, attempts_d): (String, Option<i64>, i64) = c
            .query_row(
                "SELECT status, scanned_at, attempts FROM jobs WHERE id = 3",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status_d, "error");
        assert_eq!(scanned_at_d, None);
        assert_eq!(attempts_d, 1);
    }

    #[test]
    fn open_requeues_legacy_unreadable() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("test_scraper_db_unreadable_{}", nanos));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.db");

        {
            let mut conn = open(&db_path).unwrap();
            let a = nj("a", 100);
            let b = nj("b", 100);
            let c = nj("c", 100);
            let d = nj("d", 100);
            let e = nj("e", 100);
            upsert_jobs(&mut conn, &[a, b, c, d, e]).unwrap();

            // Set up row A: status='unreadable', scanned_at NULL, attempts=2, last_error='legacy err', marks set
            conn.execute(
                "UPDATE jobs SET status = 'unreadable', scanned_at = NULL, attempts = 2, last_error = 'legacy err', starred = 1, applied_at = 9999 WHERE simplify_id = 'a'",
                [],
            )
            .unwrap();
            // Set up row B: status='unreadable', scanned_at=123
            conn.execute(
                "UPDATE jobs SET status = 'unreadable', scanned_at = 123 WHERE simplify_id = 'b'",
                [],
            )
            .unwrap();
            // Set up row C: status='rules_done', scanned_at NULL
            conn.execute(
                "UPDATE jobs SET status = 'rules_done', scanned_at = NULL WHERE simplify_id = 'c'",
                [],
            )
            .unwrap();
            // Set up row D: status='error', scanned_at NULL
            conn.execute(
                "UPDATE jobs SET status = 'error', scanned_at = NULL WHERE simplify_id = 'd'",
                [],
            )
            .unwrap();
            // Set up row E: status='gone', scanned_at NULL
            conn.execute(
                "UPDATE jobs SET status = 'gone', scanned_at = NULL WHERE simplify_id = 'e'",
                [],
            )
            .unwrap();
        }

        // First re-open: should requeue row A to pending, leaving others untouched
        {
            let conn = open(&db_path).unwrap();
            let (status_a, scanned_at_a, attempts_a, last_error_a, starred_a, applied_at_a): (
                String,
                Option<i64>,
                i64,
                Option<String>,
                i64,
                Option<i64>,
            ) = conn
                .query_row(
                    "SELECT status, scanned_at, attempts, last_error, starred, applied_at FROM jobs WHERE simplify_id = 'a'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .unwrap();
            assert_eq!(status_a, "pending");
            assert_eq!(scanned_at_a, None);
            assert_eq!(attempts_a, 2);
            assert_eq!(last_error_a.as_deref(), Some("legacy err"));
            assert_eq!(starred_a, 1);
            assert_eq!(applied_at_a, Some(9999));

            let (status_b, scanned_at_b): (String, Option<i64>) = conn
                .query_row(
                    "SELECT status, scanned_at FROM jobs WHERE simplify_id = 'b'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status_b, "unreadable");
            assert_eq!(scanned_at_b, Some(123));

            let (status_c, scanned_at_c): (String, Option<i64>) = conn
                .query_row(
                    "SELECT status, scanned_at FROM jobs WHERE simplify_id = 'c'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status_c, "rules_done");
            assert_eq!(scanned_at_c, None);

            let (status_d, scanned_at_d): (String, Option<i64>) = conn
                .query_row(
                    "SELECT status, scanned_at FROM jobs WHERE simplify_id = 'd'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status_d, "error");
            assert_eq!(scanned_at_d, None);

            let (status_e, scanned_at_e): (String, Option<i64>) = conn
                .query_row(
                    "SELECT status, scanned_at FROM jobs WHERE simplify_id = 'e'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status_e, "gone");
            assert_eq!(scanned_at_e, None);
        }

        // Second re-open: idempotent, changes nothing more
        {
            let conn2 = open(&db_path).unwrap();
            let status_a: String = conn2
                .query_row(
                    "SELECT status FROM jobs WHERE simplify_id = 'a'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status_a, "pending");

            let status_b: String = conn2
                .query_row(
                    "SELECT status FROM jobs WHERE simplify_id = 'b'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status_b, "unreadable");

            let status_c: String = conn2
                .query_row(
                    "SELECT status FROM jobs WHERE simplify_id = 'c'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status_c, "rules_done");
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_init_schema_migration_note_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
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
        ).unwrap();

        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(jobs)").unwrap();
        let cols: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(cols.contains("note"));
    }

    #[test]
    fn test_set_note_roundtrip_and_trim() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        set_note(&c, 1, Some("  my test note  ")).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows[0].note, Some("my test note".to_string()));

        set_note(&c, 1, Some("   \t  ")).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows[0].note, None);

        set_note(&c, 1, None).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows[0].note, None);
    }

    #[test]
    fn test_set_note_non_ascii_and_cap() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        let vietnamese_str = "yêu cầu kinh nghiệm – thực tập sinh khóa 2027";
        set_note(&c, 1, Some(vietnamese_str)).unwrap();
        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows[0].note, Some(vietnamese_str.to_string()));

        let long_note = "é".repeat(250);
        set_note(&c, 1, Some(&long_note)).unwrap();
        let rows = list_jobs(&c).unwrap();
        let note = rows[0].note.as_ref().unwrap();
        assert_eq!(note.chars().count(), 200);
        assert_eq!(note, &"é".repeat(200));
    }

    #[test]
    fn test_upsert_jobs_keeps_note() {
        let mut c = open_in_memory().unwrap();
        let a = nj("a", 100);
        upsert_jobs(&mut c, &[a]).unwrap();

        set_note(&c, 1, Some("keep this note")).unwrap();

        let a_newer = nj("a", 200);
        let stats = upsert_jobs(&mut c, &[a_newer]).unwrap();
        assert_eq!(stats.requeued, 1);

        let rows = list_jobs(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(rows[0].note, Some("keep this note".to_string()));
    }

    #[test]
    fn test_form_checked_sql() {
        let mut c = open_in_memory().unwrap();
        let j1 = nj("j1", 100);
        let j2 = nj("j2", 100);
        let j3 = nj("j3", 100);
        let j4 = nj("j4", 100);
        let j5 = nj("j5", 100);
        upsert_jobs(&mut c, &[j1, j2, j3, j4, j5]).unwrap();

        // j1: page is NULL (default after upsert)

        // j2: page is invalid JSON
        c.execute("UPDATE jobs SET page = '{not json' WHERE simplify_id = 'j2'", []).unwrap();

        // j3: questions is empty array
        c.execute(
            "UPDATE jobs SET page = ?1 WHERE simplify_id = 'j3'",
            params![r#"{"description":"desc","questions":[]}"#],
        ).unwrap();

        // j4: questions has 1 item
        c.execute(
            "UPDATE jobs SET page = ?1 WHERE simplify_id = 'j4'",
            params![r#"{"description":"desc","questions":[{"label":"q1","options":[]}]}"#],
        ).unwrap();

        // j5: no questions field
        c.execute(
            "UPDATE jobs SET page = ?1 WHERE simplify_id = 'j5'",
            params![r#"{"description":"desc"}"#],
        ).unwrap();

        let rows = list_jobs(&c).unwrap();
        let r1 = rows.iter().find(|r| r.job.simplify_id == "j1").unwrap();
        let r2 = rows.iter().find(|r| r.job.simplify_id == "j2").unwrap();
        let r3 = rows.iter().find(|r| r.job.simplify_id == "j3").unwrap();
        let r4 = rows.iter().find(|r| r.job.simplify_id == "j4").unwrap();
        let r5 = rows.iter().find(|r| r.job.simplify_id == "j5").unwrap();

        assert!(!r1.form_checked);
        assert!(!r2.form_checked);
        assert!(!r3.form_checked);
        assert!(r4.form_checked);
        assert!(!r5.form_checked);
    }
}
