#![allow(dead_code)]

use std::io::IsTerminal;
use std::path::Path;
use ratatui::crossterm::style::Stylize;
use serde::Serialize;
use crate::config::{Config, Profile};
use crate::db;
use crate::model::{
    Citizenship, ClassYears, Experience, Facts, Gpa,
    Sponsorship, Status,
};
use crate::text::truncate_chars;
use crate::views::{self, Tab};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Apply,
    Maybe,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub reason: String,
    pub year_confirmed: bool,
}

pub fn judge(
    status: Status,
    facts: Option<&Facts>,
    profile: &Profile,
    season_year: u16,
) -> Judgement {
    let g = profile.grad_year(season_year);

    // 1. Gone -> No, posting closed.
    if status == Status::Gone {
        return Judgement {
            verdict: Verdict::No,
            reason: "posting closed".to_string(),
            year_confirmed: false,
        };
    }

    // 2. Pending -> Maybe, not scanned yet. Unreadable -> Maybe, page unreadable. Error -> Maybe, fetch failed. facts None -> Maybe, no facts.
    match status {
        Status::Pending => {
            return Judgement {
                verdict: Verdict::Maybe,
                reason: "not scanned yet".to_string(),
                year_confirmed: false,
            };
        }
        Status::Unreadable => {
            return Judgement {
                verdict: Verdict::Maybe,
                reason: "page unreadable".to_string(),
                year_confirmed: false,
            };
        }
        Status::Error => {
            return Judgement {
                verdict: Verdict::Maybe,
                reason: "fetch failed".to_string(),
                year_confirmed: false,
            };
        }
        _ => {}
    }

    let facts = match facts {
        Some(f) => f,
        None => {
            return Judgement {
                verdict: Verdict::Maybe,
                reason: "no facts".to_string(),
                year_confirmed: false,
            };
        }
    };

    // year_confirmed = Grad(set) containing g, or Any with evidence.is_some()
    let year_confirmed = match &facts.class_years.value {
        ClassYears::Grad(set) => set.contains(&g),
        ClassYears::Any => facts.class_years.evidence.is_some(),
        ClassYears::Unknown => false,
    };

    // 3. No rules, in this order:
    // - citizenship Required -> US citizenship/clearance required: "{snippet}"
    if matches!(facts.citizenship.value, Citizenship::Required) {
        let snippet = facts
            .citizenship
            .evidence
            .as_ref()
            .map(|e| e.snippet.as_str())
            .unwrap_or("");
        return Judgement {
            verdict: Verdict::No,
            reason: format!(r#"US citizenship/clearance required: "{snippet}""#),
            year_confirmed,
        };
    }

    // - sponsorship NotOffered and needs_sponsorship -> no sponsorship: "{snippet}"
    if profile.needs_sponsorship && matches!(facts.sponsorship.value, Sponsorship::NotOffered) {
        let snippet = facts
            .sponsorship
            .evidence
            .as_ref()
            .map(|e| e.snippet.as_str())
            .unwrap_or("");
        return Judgement {
            verdict: Verdict::No,
            reason: format!(r#"no sponsorship: "{snippet}""#),
            year_confirmed,
        };
    }

    // - class_years Grad(set) without g -> class years {min}-{max} (you: {g})
    if let ClassYears::Grad(ref set) = facts.class_years.value {
        if !set.contains(&g) {
            let min = set.first().copied().unwrap_or(0);
            let max = set.last().copied().unwrap_or(0);
            return Judgement {
                verdict: Verdict::No,
                reason: format!("class years {min}-{max} (you: {g})"),
                year_confirmed,
            };
        }
    }

    // - gpa Min(x) and profile.gpa == Some(p) with p < x -> GPA min {x} (you: {p})
    if let Gpa::Min(x) = facts.gpa.value {
        if let Some(p) = profile.gpa {
            if p < x {
                return Judgement {
                    verdict: Verdict::No,
                    reason: format!("GPA min {x} (you: {p})"),
                    year_confirmed,
                };
            }
        }
    }

    // - experience Required and not has_prior_internship -> prior experience required: "{snippet}"
    if !profile.has_prior_internship && matches!(facts.experience.value, Experience::Required) {
        let snippet = facts
            .experience
            .evidence
            .as_ref()
            .map(|e| e.snippet.as_str())
            .unwrap_or("");
        return Judgement {
            verdict: Verdict::No,
            reason: format!(r#"prior experience required: "{snippet}""#),
            year_confirmed,
        };
    }

    // 4. Maybe reasons:
    let mut reasons = Vec::new();

    // - any Unknown -> unclear: {unknown_names joined ", "}
    if facts.has_unknown() {
        reasons.push(format!("unclear: {}", facts.unknown_names().join(", ")));
    }

    // - sponsorship Asked and needs_sponsorship -> asks about sponsorship
    if profile.needs_sponsorship && matches!(facts.sponsorship.value, Sponsorship::Asked) {
        reasons.push("asks about sponsorship".to_string());
    }

    // - gpa Asked -> asks for GPA
    if matches!(facts.gpa.value, Gpa::Asked) {
        reasons.push("asks for GPA".to_string());
    }

    // - gpa Min(x) with no profile GPA -> GPA min {x} (you have none yet)
    if let Gpa::Min(x) = facts.gpa.value {
        if profile.gpa.is_none() {
            reasons.push(format!("GPA min {x} (you have none yet)"));
        }
    }

    // - experience Preferred and no prior internship -> prior experience preferred
    if !profile.has_prior_internship && matches!(facts.experience.value, Experience::Preferred) {
        reasons.push("prior experience preferred".to_string());
    }

    if !reasons.is_empty() {
        return Judgement {
            verdict: Verdict::Maybe,
            reason: reasons.join("; "),
            year_confirmed,
        };
    }

    // 5. Otherwise -> Apply, no blockers found.
    Judgement {
        verdict: Verdict::Apply,
        reason: "no blockers found".to_string(),
        year_confirmed,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListFlags {
    pub all: bool,
    pub starred: bool,
    pub applied: bool,
    pub new: bool,
}

pub fn select_rows<'a>(rows: &'a [db::Row], flags: ListFlags) -> Vec<&'a db::Row> {
    rows.iter()
        .filter(|row| {
            if flags.applied {
                views::in_tab(row, Tab::Applied)
            } else if flags.starred {
                views::in_tab(row, Tab::Starred)
            } else if flags.all {
                row.active && (views::in_tab(row, Tab::Jobs) || views::in_tab(row, Tab::Hidden))
            } else {
                views::in_tab(row, Tab::Jobs)
            }
        })
        .filter(|row| !flags.new || row.is_new)
        .collect()
}

pub fn render_table(view: &[views::ViewRow], color: bool) -> String {
    let header = format!(
        "{:<4} {:<7} {:<5} {:<20} {:<40} {:<60} {}",
        "MARK", "VERDICT", "YEAR", "COMPANY", "TITLE", "REASON", "URL"
    );

    let mut lines = Vec::with_capacity(view.len() + 1);
    lines.push(header);

    for vr in view {
        let mut mark_text = String::new();
        if vr.row.marks.starred {
            mark_text.push('★');
        }
        if vr.row.marks.applied_at.is_some() {
            mark_text.push('✓');
        }
        if vr.row.is_new {
            mark_text.push('N');
        }

        let mark_char_count = mark_text.chars().count();
        let mark_pad = 4usize.saturating_sub(mark_char_count);
        let formatted_mark = if color {
            if mark_char_count > 0 {
                format!("{}{}", mark_text.cyan(), " ".repeat(mark_pad))
            } else {
                " ".repeat(4)
            }
        } else {
            format!("{}{}", mark_text, " ".repeat(mark_pad))
        };

        let formatted_verdict = if color {
            match vr.judgement.verdict {
                Verdict::Apply => format!("{}  ", "apply".green()),
                Verdict::Maybe => format!("{}  ", "maybe".yellow()),
                Verdict::No => format!("{}     ", "no".red()),
            }
        } else {
            match vr.judgement.verdict {
                Verdict::Apply => format!("{:<7}", "apply"),
                Verdict::Maybe => format!("{:<7}", "maybe"),
                Verdict::No => format!("{:<7}", "no"),
            }
        };

        let year_str = if vr.judgement.year_confirmed {
            "yes"
        } else {
            "year?"
        };
        let formatted_year = format!("{:<5}", year_str);
        let trunc_company = truncate_chars(&vr.row.job.company, 20);
        let formatted_company = format!("{:<20}", trunc_company);
        let trunc_title = truncate_chars(&vr.row.job.title, 40);
        let formatted_title = format!("{:<40}", trunc_title);
        let trunc_reason = truncate_chars(&vr.judgement.reason, 60);
        let formatted_reason = format!("{:<60}", trunc_reason);

        lines.push(format!(
            "{formatted_mark} {formatted_verdict} {formatted_year} {formatted_company} {formatted_title} {formatted_reason} {}",
            vr.row.job.url
        ));
    }

    lines.join("\n")
}

#[derive(Serialize)]
pub struct JsonOutput<'a> {
    pub verdict: Verdict,
    pub year_confirmed: bool,
    pub company: &'a str,
    pub title: &'a str,
    pub url: &'a str,
    pub reason: &'a str,
    pub status: Status,
    pub facts: Option<&'a Facts>,
    pub starred: bool,
    pub hidden: bool,
    pub applied_at: Option<i64>,
    pub new: bool,
}

/// Writes to stdout, treating a closed pipe (`scraper list | head`) as success instead of panicking.
fn write_stdout(s: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_all(s.as_bytes()).and_then(|_| out.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

pub fn list(
    cfg: &Config,
    db_path: &Path,
    year: Option<u8>,
    verdict: Option<String>,
    all: bool,
    json: bool,
    starred: bool,
    applied: bool,
    new: bool,
) -> anyhow::Result<()> {
    let profile = if let Some(n) = year {
        if !(1..=4).contains(&n) {
            anyhow::bail!("year must be between 1 and 4, got {n}");
        }
        let mut p = cfg.profile.clone();
        p.class_year = n;
        p.expected_graduation = None;
        p
    } else {
        cfg.profile.clone()
    };

    let target_verdict = if let Some(ref v) = verdict {
        match v.to_ascii_lowercase().as_str() {
            "apply" => Some(Verdict::Apply),
            "maybe" => Some(Verdict::Maybe),
            "no" => Some(Verdict::No),
            other => anyhow::bail!("invalid verdict '{other}': expected apply, maybe, or no"),
        }
    } else {
        None
    };

    let conn = db::open(db_path)?;
    let rows = db::list_jobs(&conn)?;

    let flags = ListFlags {
        all,
        starred,
        applied,
        new,
    };

    let selected_rows = select_rows(&rows, flags);

    let mut view_rows: Vec<views::ViewRow> = selected_rows
        .into_iter()
        .map(|row| {
            let judgement = judge(row.status, row.facts.as_ref(), &profile, cfg.season_year);
            views::ViewRow { row, judgement }
        })
        .collect();

    let a_count = view_rows.iter().filter(|i| i.judgement.verdict == Verdict::Apply).count();
    let m_count = view_rows.iter().filter(|i| i.judgement.verdict == Verdict::Maybe).count();
    let n_count = view_rows.iter().filter(|i| i.judgement.verdict == Verdict::No).count();

    // If --verdict is given it filters; otherwise Jobs uses ApplyMaybe, while Starred/Applied/All show all verdicts
    view_rows.retain(|vr| {
        if let Some(tv) = target_verdict {
            vr.judgement.verdict == tv
        } else if starred || applied || all {
            true
        } else {
            matches!(vr.judgement.verdict, Verdict::Apply | Verdict::Maybe)
        }
    });

    if applied {
        view_rows.sort_by_key(|vr| std::cmp::Reverse(vr.row.marks.applied_at));
    } else {
        view_rows.sort_by_key(|vr| {
            (
                !vr.row.marks.starred,
                vr.judgement.verdict,
                std::cmp::Reverse(vr.row.date_posted),
            )
        });
    }

    if json {
        let json_items: Vec<JsonOutput> = view_rows
            .iter()
            .map(|vr| JsonOutput {
                verdict: vr.judgement.verdict,
                year_confirmed: vr.judgement.year_confirmed,
                company: &vr.row.job.company,
                title: &vr.row.job.title,
                url: &vr.row.job.url,
                reason: &vr.judgement.reason,
                status: vr.row.status,
                facts: vr.row.facts.as_ref(),
                starred: vr.row.marks.starred,
                hidden: vr.row.marks.hidden,
                applied_at: vr.row.marks.applied_at,
                new: vr.row.is_new,
            })
            .collect();

        write_stdout(&format!("{}
", serde_json::to_string_pretty(&json_items)?))?;
    } else {
        let color = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").map_or(true, |v| v.is_empty());
        let table = render_table(&view_rows, color);
        write_stdout(&format!("{table}
{a_count} apply, {m_count} maybe, {n_count} no
"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use crate::model::{
        Citizenship, ClassYears, Evidence, Experience, Fact, Gpa,
        NewJob, PortalPage, Provider, Source, Sponsorship,
    };

    fn profile() -> Profile {
        Profile {
            class_year: 1,
            expected_graduation: None,
            gpa: None,
            needs_sponsorship: true,
            has_prior_internship: false,
        }
    }

    fn clean() -> Facts {
        Facts {
            class_years: Fact {
                value: ClassYears::Any,
                evidence: None,
            },
            gpa: Fact {
                value: Gpa::NotMentioned,
                evidence: None,
            },
            sponsorship: Fact {
                value: Sponsorship::NotMentioned,
                evidence: None,
            },
            citizenship: Fact {
                value: Citizenship::NotMentioned,
                evidence: None,
            },
            experience: Fact {
                value: Experience::NotMentioned,
                evidence: None,
            },
        }
    }

    #[test]
    fn clean_is_apply() {
        let j = judge(Status::RulesDone, Some(&clean()), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::Apply);
        assert_eq!(j.reason, "no blockers found");
        assert!(!j.year_confirmed);
    }

    #[test]
    fn citizenship_is_no() {
        let mut f = clean();
        f.citizenship = Fact {
            value: Citizenship::Required,
            evidence: Some(Evidence {
                source: Source::Description,
                snippet: "Must be a US citizen".to_string(),
            }),
        };
        let j = judge(Status::RulesDone, Some(&f), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::No);
        assert!(j.reason.starts_with("US citizenship/clearance required:"));
        assert!(j.reason.contains("Must be a US citizen"));
    }

    #[test]
    fn sponsorship_not_offered_only_blocks_if_needed() {
        let mut f = clean();
        f.sponsorship = Fact {
            value: Sponsorship::NotOffered,
            evidence: Some(Evidence {
                source: Source::Description,
                snippet: "No CPT/OPT".to_string(),
            }),
        };
        let j = judge(Status::RulesDone, Some(&f), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::No);
        assert!(j.reason.starts_with("no sponsorship:"));

        let mut p = profile();
        p.needs_sponsorship = false;
        let j2 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j2.verdict, Verdict::Apply);
    }

    #[test]
    fn asked_sponsorship_is_maybe() {
        let mut f = clean();
        f.sponsorship = Fact {
            value: Sponsorship::Asked,
            evidence: None,
        };
        let j = judge(Status::RulesDone, Some(&f), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::Maybe);
        assert!(j.reason.contains("asks about sponsorship"));
    }

    #[test]
    fn gpa_min_only_blocks_when_owner_has_gpa() {
        let mut f = clean();
        f.gpa = Fact {
            value: Gpa::Min(3.0),
            evidence: None,
        };
        let mut p = profile();
        p.gpa = None;
        let j = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j.verdict, Verdict::Maybe);
        assert_eq!(j.reason, "GPA min 3 (you have none yet)");

        p.gpa = Some(2.8);
        let j2 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j2.verdict, Verdict::No);
        assert_eq!(j2.reason, "GPA min 3 (you: 2.8)");

        p.gpa = Some(3.5);
        let j3 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j3.verdict, Verdict::Apply);
    }

    #[test]
    fn year_override_changes_verdict() {
        let mut f = clean();
        let mut years = BTreeSet::new();
        years.insert(2028);
        years.insert(2029);
        f.class_years = Fact {
            value: ClassYears::Grad(years),
            evidence: None,
        };
        let mut p = profile();
        p.class_year = 1;
        let j = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j.verdict, Verdict::No);
        assert_eq!(j.reason, "class years 2028-2029 (you: 2030)");
        assert!(!j.year_confirmed);

        p.class_year = 2;
        let j2 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j2.verdict, Verdict::Apply);
        assert!(j2.year_confirmed);
    }

    #[test]
    fn experience_rules() {
        let mut f = clean();
        f.experience = Fact {
            value: Experience::Required,
            evidence: Some(Evidence {
                source: Source::Description,
                snippet: "Must have 1 internship".to_string(),
            }),
        };
        let mut p = profile();
        p.has_prior_internship = false;
        let j = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j.verdict, Verdict::No);
        assert!(j.reason.starts_with("prior experience required:"));

        p.has_prior_internship = true;
        let j2 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j2.verdict, Verdict::Apply);

        f.experience = Fact {
            value: Experience::Preferred,
            evidence: None,
        };
        p.has_prior_internship = false;
        let j3 = judge(Status::RulesDone, Some(&f), &p, 2027);
        assert_eq!(j3.verdict, Verdict::Maybe);
        assert!(j3.reason.contains("prior experience preferred"));
    }

    #[test]
    fn unknowns_listed() {
        let f = Facts::unknown();
        let j = judge(Status::RulesDone, Some(&f), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::Maybe);
        assert_eq!(
            j.reason,
            "unclear: class_years, gpa, sponsorship, citizenship, experience"
        );
    }

    #[test]
    fn status_overrides() {
        let p = profile();
        let j_gone = judge(Status::Gone, None, &p, 2027);
        assert_eq!(j_gone.verdict, Verdict::No);
        assert_eq!(j_gone.reason, "posting closed");

        let j_unreadable = judge(Status::Unreadable, None, &p, 2027);
        assert_eq!(j_unreadable.verdict, Verdict::Maybe);
        assert_eq!(j_unreadable.reason, "page unreadable");

        let j_pending = judge(Status::Pending, None, &p, 2027);
        assert_eq!(j_pending.verdict, Verdict::Maybe);
        assert_eq!(j_pending.reason, "not scanned yet");

        let j_error = judge(Status::Error, None, &p, 2027);
        assert_eq!(j_error.verdict, Verdict::Maybe);
        assert_eq!(j_error.reason, "fetch failed");

        let j_no_facts = judge(Status::RulesDone, None, &p, 2027);
        assert_eq!(j_no_facts.verdict, Verdict::Maybe);
        assert_eq!(j_no_facts.reason, "no facts");
    }

    #[test]
    fn first_no_wins() {
        let mut f = clean();
        f.citizenship = Fact {
            value: Citizenship::Required,
            evidence: Some(Evidence {
                source: Source::Description,
                snippet: "US citizenship required".to_string(),
            }),
        };
        f.experience = Fact {
            value: Experience::Required,
            evidence: Some(Evidence {
                source: Source::Description,
                snippet: "Must have internship experience".to_string(),
            }),
        };
        let j = judge(Status::RulesDone, Some(&f), &profile(), 2027);
        assert_eq!(j.verdict, Verdict::No);
        assert!(j.reason.starts_with("US citizenship/clearance required:"));
    }

    #[test]
    fn list_year_validation() {
        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = false
            has_prior_internship = false
        "#;
        let cfg = crate::config::parse(toml_str).unwrap();
        let db_path = Path::new("dummy.db");

        let err0 = list(&cfg, db_path, Some(0), None, false, false, false, false, false).unwrap_err().to_string();
        assert!(err0.contains("year must be between 1 and 4"));

        let err5 = list(&cfg, db_path, Some(5), None, false, false, false, false, false).unwrap_err().to_string();
        assert!(err5.contains("year must be between 1 and 4"));
    }

    #[test]
    fn list_verdict_validation() {
        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = false
            has_prior_internship = false
        "#;
        let cfg = crate::config::parse(toml_str).unwrap();
        let db_path = Path::new("dummy.db");

        let err = list(&cfg, db_path, None, Some("foo".to_string()), false, false, false, false, false).unwrap_err().to_string();
        assert!(err.contains("invalid verdict 'foo'"));
    }

    #[test]
    fn list_execution_table_and_json() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = true
            has_prior_internship = false
        "#;
        let cfg = crate::config::parse(toml_str).unwrap();

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("test_list_cmd_{}", nanos));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.db");

        {
            let mut conn = db::open(&db_path).unwrap();
            let nj1 = NewJob {
                simplify_id: "job1".to_string(),
                company: "CleanCorp".to_string(),
                title: "SWE Intern".to_string(),
                category: "Software".to_string(),
                url: "https://example.com/job1".to_string(),
                provider: Provider::Other,
                locations: vec!["Remote".to_string()],
                date_posted: 100,
                date_updated: 100,
            };
            let nj2 = NewJob {
                simplify_id: "job2".to_string(),
                company: "NoSponsorCo".to_string(),
                title: "ML Intern".to_string(),
                category: "Software".to_string(),
                url: "https://example.com/job2".to_string(),
                provider: Provider::Other,
                locations: vec!["Remote".to_string()],
                date_posted: 101,
                date_updated: 101,
            };
            db::upsert_jobs(&mut conn, &[nj1, nj2]).unwrap();

            let page = PortalPage {
                description: "Clean job".to_string(),
                questions: vec![],
            };
            db::save_page(&conn, 1, &page, &clean()).unwrap();

            let mut no_facts = clean();
            no_facts.sponsorship = Fact {
                value: Sponsorship::NotOffered,
                evidence: Some(Evidence {
                    source: Source::Description,
                    snippet: "No sponsorship".to_string(),
                }),
            };
            db::save_page(&conn, 2, &page, &no_facts).unwrap();
        }

        // Test default table run (shows apply, hides no)
        assert!(list(&cfg, &db_path, None, None, false, false, false, false, false).is_ok());

        // Test with --all
        assert!(list(&cfg, &db_path, None, None, true, false, false, false, false).is_ok());

        // Test with --verdict no
        assert!(list(&cfg, &db_path, None, Some("no".to_string()), false, false, false, false, false).is_ok());

        // Test with --json
        assert!(list(&cfg, &db_path, None, None, false, true, false, false, false).is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn make_test_row(
        id: i64,
        company: &str,
        title: &str,
        active: bool,
        marks: crate::db::Marks,
        is_new: bool,
        status: Status,
    ) -> crate::db::Row {
        crate::db::Row {
            job: crate::model::Job {
                id,
                simplify_id: format!("job_{id}"),
                company: company.to_string(),
                title: title.to_string(),
                url: format!("https://example.com/job/{id}"),
                provider: Provider::Other,
            },
            status,
            facts: Some(clean()),
            date_posted: 100,
            active,
            marks,
            is_new,
        }
    }

    #[test]
    fn no_color_when_disabled() {
        let row = make_test_row(1, "Acme", "SWE", true, crate::db::Marks::default(), false, Status::RulesDone);
        let judgement = Judgement {
            verdict: Verdict::Apply,
            reason: "no blockers found".to_string(),
            year_confirmed: true,
        };
        let view = [crate::views::ViewRow { row: &row, judgement }];
        let out = render_table(&view, false);
        assert!(!out.contains('\x1b'));
        assert!(out.contains("apply"));
    }

    #[test]
    fn color_when_enabled() {
        let row = make_test_row(1, "Acme", "SWE", true, crate::db::Marks::default(), false, Status::RulesDone);
        let judgement = Judgement {
            verdict: Verdict::Apply,
            reason: "no blockers found".to_string(),
            year_confirmed: true,
        };
        let view = [crate::views::ViewRow { row: &row, judgement }];
        let out = render_table(&view, true);
        assert!(out.contains('\x1b'));
        assert!(out.contains("apply"));
    }

    #[test]
    fn marks_column() {
        let row1 = make_test_row(
            1,
            "Acme",
            "SWE",
            true,
            crate::db::Marks { starred: true, hidden: false, applied_at: None },
            true,
            Status::RulesDone,
        );
        let row2 = make_test_row(
            2,
            "Beta",
            "MLE",
            true,
            crate::db::Marks { starred: false, hidden: false, applied_at: Some(100) },
            false,
            Status::RulesDone,
        );
        let j1 = Judgement {
            verdict: Verdict::Apply,
            reason: "no blockers found".to_string(),
            year_confirmed: true,
        };
        let j2 = Judgement {
            verdict: Verdict::Maybe,
            reason: "not scanned yet".to_string(),
            year_confirmed: false,
        };
        let view = [
            crate::views::ViewRow { row: &row1, judgement: j1 },
            crate::views::ViewRow { row: &row2, judgement: j2 },
        ];
        let out = render_table(&view, false);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() >= 3);
        assert!(lines[1].starts_with("★N"));
        assert!(lines[2].starts_with("✓"));
    }

    #[test]
    fn hidden_excluded_by_default() {
        let normal = make_test_row(1, "Acme", "SWE", true, crate::db::Marks::default(), false, Status::RulesDone);
        let hidden = make_test_row(
            2,
            "HideCorp",
            "SWE",
            true,
            crate::db::Marks { starred: false, hidden: true, applied_at: None },
            false,
            Status::RulesDone,
        );
        let rows = [normal, hidden];

        let default_flags = ListFlags::default();
        let selected_default = select_rows(&rows, default_flags);
        assert_eq!(selected_default.len(), 1);
        assert_eq!(selected_default[0].job.simplify_id, "job_1");

        let all_flags = ListFlags { all: true, ..Default::default() };
        let selected_all = select_rows(&rows, all_flags);
        assert_eq!(selected_all.len(), 2);
    }

    #[test]
    fn applied_flag_shows_closed_posting() {
        let applied_closed = make_test_row(
            1,
            "OldCorp",
            "SWE",
            false,
            crate::db::Marks { starred: false, hidden: false, applied_at: Some(100) },
            false,
            Status::Gone,
        );
        let rows = [applied_closed];
        let flags = ListFlags { applied: true, ..Default::default() };
        let selected = select_rows(&rows, flags);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].job.simplify_id, "job_1");
    }

    #[test]
    fn json_has_new_fields() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = false
            has_prior_internship = false
        "#;
        let cfg = crate::config::parse(toml_str).unwrap();

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("test_json_fields_{}", nanos));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.db");

        {
            let mut conn = db::open(&db_path).unwrap();
            let nj1 = NewJob {
                simplify_id: "job1".to_string(),
                company: "CleanCorp".to_string(),
                title: "SWE Intern".to_string(),
                category: "Software".to_string(),
                url: "https://example.com/job1".to_string(),
                provider: Provider::Other,
                locations: vec!["Remote".to_string()],
                date_posted: 100,
                date_updated: 100,
            };
            db::upsert_jobs(&mut conn, &[nj1]).unwrap();
            db::set_marks(
                &conn,
                1,
                crate::db::Marks {
                    starred: true,
                    hidden: false,
                    applied_at: Some(12345),
                },
            )
            .unwrap();
        }

        assert!(list(&cfg, &db_path, None, None, false, true, false, true, false).is_ok());

        let conn = db::open(&db_path).unwrap();
        let rows = db::list_jobs(&conn).unwrap();
        let flags = ListFlags { applied: true, ..Default::default() };
        let selected = select_rows(&rows, flags);
        let j = judge(selected[0].status, selected[0].facts.as_ref(), &cfg.profile, cfg.season_year);
        let view_row = crate::views::ViewRow { row: selected[0], judgement: j };
        let json_val = serde_json::to_value(&JsonOutput {
            verdict: view_row.judgement.verdict,
            year_confirmed: view_row.judgement.year_confirmed,
            company: &view_row.row.job.company,
            title: &view_row.row.job.title,
            url: &view_row.row.job.url,
            reason: &view_row.judgement.reason,
            status: view_row.row.status,
            facts: view_row.row.facts.as_ref(),
            starred: view_row.row.marks.starred,
            hidden: view_row.row.marks.hidden,
            applied_at: view_row.row.marks.applied_at,
            new: view_row.row.is_new,
        })
        .unwrap();

        assert!(json_val.get("starred").is_some());
        assert!(json_val.get("hidden").is_some());
        assert!(json_val.get("applied_at").is_some());
        assert!(json_val.get("new").is_some());

        assert!(json_val.get("verdict").is_some());
        assert!(json_val.get("year_confirmed").is_some());
        assert!(json_val.get("company").is_some());
        assert!(json_val.get("title").is_some());
        assert!(json_val.get("url").is_some());
        assert!(json_val.get("reason").is_some());
        assert!(json_val.get("status").is_some());
        assert!(json_val.get("facts").is_some());

        assert_eq!(json_val["starred"], true);
        assert_eq!(json_val["hidden"], false);
        assert_eq!(json_val["applied_at"], 12345);
        assert_eq!(json_val["new"], false);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
