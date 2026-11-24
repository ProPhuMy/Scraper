#![allow(dead_code)]

use crate::config::Profile;
use crate::db::{Marks, Row};
use crate::verdict::{judge, Judgement, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Jobs,
    Starred,
    Applied,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictFilter {
    ApplyMaybe,
    Apply,
    Maybe,
    No,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKey {
    Star,
    Hide,
    Applied,
}

pub fn in_tab(row: &Row, tab: Tab) -> bool {
    let applied = row.marks.applied_at.is_some();
    match tab {
        Tab::Applied => applied,
        Tab::Hidden => row.marks.hidden && !applied,
        Tab::Starred => row.marks.starred && !applied && !row.marks.hidden,
        Tab::Jobs => row.active && !applied && !row.marks.hidden,
    }
}

pub fn toggle(mut marks: Marks, key: MarkKey, now: i64) -> Result<Marks, &'static str> {
    match key {
        MarkKey::Star => {
            marks.starred = !marks.starred;
            if marks.starred {
                marks.hidden = false;
            }
            Ok(marks)
        }
        MarkKey::Hide => {
            if marks.applied_at.is_some() {
                return Err("applied jobs can't be hidden");
            }
            marks.hidden = !marks.hidden;
            if marks.hidden {
                marks.starred = false;
            }
            Ok(marks)
        }
        MarkKey::Applied => {
            marks.applied_at = if marks.applied_at.is_some() {
                None
            } else {
                Some(now)
            };
            Ok(marks)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewRow<'a> {
    pub row: &'a Row,
    pub judgement: Judgement,
}

pub fn build_view<'a>(
    rows: &'a [Row],
    tab: Tab,
    filter: VerdictFilter,
    search: &str,
    profile: &Profile,
    season_year: u16,
) -> Vec<ViewRow<'a>> {
    let needle = search.to_lowercase();

    let mut result: Vec<ViewRow<'a>> = rows
        .iter()
        .filter(|row| in_tab(row, tab))
        .filter(|row| {
            if needle.is_empty() {
                true
            } else {
                row.job.company.to_lowercase().contains(&needle)
                    || row.job.title.to_lowercase().contains(&needle)
            }
        })
        .map(|row| {
            let judgement = judge(row.status, row.facts.as_ref(), profile, season_year);
            ViewRow { row, judgement }
        })
        .filter(|vr| {
            if tab == Tab::Jobs {
                match filter {
                    VerdictFilter::ApplyMaybe => {
                        matches!(vr.judgement.verdict, Verdict::Apply | Verdict::Maybe)
                    }
                    VerdictFilter::Apply => vr.judgement.verdict == Verdict::Apply,
                    VerdictFilter::Maybe => vr.judgement.verdict == Verdict::Maybe,
                    VerdictFilter::No => vr.judgement.verdict == Verdict::No,
                    VerdictFilter::All => true,
                }
            } else {
                true
            }
        })
        .collect();

    if tab == Tab::Applied {
        result.sort_by_key(|vr| std::cmp::Reverse(vr.row.marks.applied_at));
    } else {
        result.sort_by_key(|vr| {
            (
                !vr.row.marks.starred,
                vr.judgement.verdict,
                std::cmp::Reverse(vr.row.date_posted),
            )
        });
    }

    result
}

pub fn tab_count(
    rows: &[Row],
    tab: Tab,
    filter: VerdictFilter,
    search: &str,
    profile: &Profile,
    season_year: u16,
) -> usize {
    build_view(rows, tab, filter, search, profile, season_year).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use crate::model::{
        Citizenship, ClassYears, Fact, Facts, Gpa, Job, Provider, Sponsorship, Status, Experience,
    };

    fn test_profile() -> Profile {
        Profile {
            class_year: 1,
            expected_graduation: None,
            gpa: None,
            needs_sponsorship: true,
            has_prior_internship: false,
        }
    }

    fn clean_facts() -> Facts {
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

    fn make_row(
        id: i64,
        company: &str,
        title: &str,
        active: bool,
        marks: Marks,
        status: Status,
        facts: Option<Facts>,
        date_posted: i64,
    ) -> Row {
        Row {
            job: Job {
                id,
                simplify_id: format!("job_{id}"),
                company: company.to_string(),
                title: title.to_string(),
                url: format!("https://example.com/job/{id}"),
                provider: Provider::Other,
            },
            status,
            facts,
            date_posted,
            active,
            marks,
            is_new: false,
            locations: vec![],
            asks_location: false,
        }
    }

    #[test]
    fn tab_membership_table() {
        // (active, starred, hidden, applied) -> expected tabs
        // (true,false,false,None) -> [Jobs]; (true,true,false,None) -> [Jobs, Starred]; (false,true,false,None) -> [Starred]
        // (true,false,true,None) -> [Hidden]; (true,true,false,Some(1)) -> [Applied]; (true,false,true,Some(1)) -> [Applied]; (false,false,false,None) -> []
        let all_tabs = [Tab::Jobs, Tab::Starred, Tab::Applied, Tab::Hidden];
        let check = |active: bool, starred: bool, hidden: bool, applied_at: Option<i64>, expected: &[Tab]| {
            let row = make_row(
                1,
                "Acme",
                "Engineer",
                active,
                Marks {
                    starred,
                    hidden,
                    applied_at,
                },
                Status::RulesDone,
                Some(clean_facts()),
                100,
            );
            let matching: Vec<Tab> = all_tabs
                .iter()
                .copied()
                .filter(|&tab| in_tab(&row, tab))
                .collect();
            assert_eq!(matching, expected, "Failed for active={active}, starred={starred}, hidden={hidden}, applied={applied_at:?}");
        };

        check(true, false, false, None, &[Tab::Jobs]);
        check(true, true, false, None, &[Tab::Jobs, Tab::Starred]);
        check(false, true, false, None, &[Tab::Starred]);
        check(true, false, true, None, &[Tab::Hidden]);
        check(true, true, false, Some(1), &[Tab::Applied]);
        check(true, false, true, Some(1), &[Tab::Applied]);
        check(false, false, false, None, &[]);
    }

    #[test]
    fn toggle_rules() {
        // star on hidden -> starred=true, hidden=false; hide on starred -> hidden=true, starred=false
        // hide on applied -> Err("applied jobs can't be hidden"); applied twice -> back to None, starred preserved
        let now = 1000;

        // star on hidden -> starred=true, hidden=false
        let m1 = Marks {
            starred: false,
            hidden: true,
            applied_at: None,
        };
        let res1 = toggle(m1, MarkKey::Star, now).unwrap();
        assert_eq!(res1, Marks {
            starred: true,
            hidden: false,
            applied_at: None,
        });

        // hide on starred -> hidden=true, starred=false
        let m2 = Marks {
            starred: true,
            hidden: false,
            applied_at: None,
        };
        let res2 = toggle(m2, MarkKey::Hide, now).unwrap();
        assert_eq!(res2, Marks {
            starred: false,
            hidden: true,
            applied_at: None,
        });

        // hide on applied -> Err("applied jobs can't be hidden")
        let m3 = Marks {
            starred: false,
            hidden: false,
            applied_at: Some(500),
        };
        let res3 = toggle(m3, MarkKey::Hide, now);
        assert_eq!(res3, Err("applied jobs can't be hidden"));

        // applied twice -> back to None, starred preserved
        let m4 = Marks {
            starred: true,
            hidden: false,
            applied_at: None,
        };
        let res4a = toggle(m4, MarkKey::Applied, now).unwrap();
        assert_eq!(res4a, Marks {
            starred: true,
            hidden: false,
            applied_at: Some(now),
        });
        let res4b = toggle(res4a, MarkKey::Applied, now + 10).unwrap();
        assert_eq!(res4b, Marks {
            starred: true,
            hidden: false,
            applied_at: None,
        });
    }

    #[test]
    fn applied_tab_ignores_verdict_filter() {
        // applied row whose status is Gone (verdict No) appears in build_view(Applied, ApplyMaybe, ...)
        let row = make_row(
            1,
            "Acme",
            "Engineer",
            false,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(100),
            },
            Status::Gone,
            None,
            100,
        );
        let profile = test_profile();
        let rows = [row];
        let view = build_view(&rows, Tab::Applied, VerdictFilter::ApplyMaybe, "", &profile, 2027);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].judgement.verdict, Verdict::No);
    }

    #[test]
    fn jobs_tab_default_filter_hides_no() {
        // Jobs + ApplyMaybe excludes a No row; Jobs + All includes it
        let mut facts = clean_facts();
        facts.citizenship.value = Citizenship::Required;
        let row = make_row(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(facts),
            100,
        );
        let profile = test_profile();

        let rows_default = [row.clone()];
        let view_default = build_view(&rows_default, Tab::Jobs, VerdictFilter::ApplyMaybe, "", &profile, 2027);
        assert_eq!(view_default.len(), 0);

        let rows_all = [row];
        let view_all = build_view(&rows_all, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        assert_eq!(view_all.len(), 1);
        assert_eq!(view_all[0].judgement.verdict, Verdict::No);
    }

    #[test]
    fn search_applies_to_all_tabs() {
        // search "neur" matches company "Neuralink" in Starred tab, case-insensitive; non-match excluded
        let row1 = make_row(
            1,
            "Neuralink",
            "Engineer",
            true,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(clean_facts()),
            100,
        );
        let row2 = make_row(
            2,
            "Anthropic",
            "Researcher",
            true,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(clean_facts()),
            100,
        );
        let profile = test_profile();
        let rows = [row1, row2];
        let view = build_view(&rows, Tab::Starred, VerdictFilter::All, "neur", &profile, 2027);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].row.job.company, "Neuralink");
    }

    #[test]
    fn sort_starred_then_verdict_then_date() {
        // [maybe new, apply old, starred no] in Jobs/All -> starred first, then apply, then maybe
        let mut maybe_facts = clean_facts();
        maybe_facts.sponsorship.value = Sponsorship::Asked;

        let mut no_facts = clean_facts();
        no_facts.citizenship.value = Citizenship::Required;

        let maybe_new = make_row(
            1,
            "Acme",
            "Maybe New",
            true,
            Marks {
                starred: false,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(maybe_facts),
            200,
        );
        let apply_old = make_row(
            2,
            "Acme",
            "Apply Old",
            true,
            Marks {
                starred: false,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(clean_facts()),
            100,
        );
        let starred_no = make_row(
            3,
            "Acme",
            "Starred No",
            true,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(no_facts),
            50,
        );

        let profile = test_profile();
        let rows = [maybe_new, apply_old, starred_no];
        let view = build_view(
            &rows,
            Tab::Jobs,
            VerdictFilter::All,
            "",
            &profile,
            2027,
        );
        let titles: Vec<&str> = view.iter().map(|v| v.row.job.title.as_str()).collect();
        assert_eq!(titles, vec!["Starred No", "Apply Old", "Maybe New"]);
    }

    #[test]
    fn applied_sorted_by_applied_at_desc() {
        let row_old = make_row(
            1,
            "Acme",
            "Applied Old",
            false,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(100),
            },
            Status::RulesDone,
            Some(clean_facts()),
            500,
        );
        let row_new = make_row(
            2,
            "Acme",
            "Applied New",
            false,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(200),
            },
            Status::RulesDone,
            Some(clean_facts()),
            300,
        );
        let profile = test_profile();
        let rows = [row_old, row_new];
        let view = build_view(&rows, Tab::Applied, VerdictFilter::All, "", &profile, 2027);
        let titles: Vec<&str> = view.iter().map(|v| v.row.job.title.as_str()).collect();
        assert_eq!(titles, vec!["Applied New", "Applied Old"]);
    }

    #[test]
    fn year_changes_verdict() {
        // class_years Grad({2028,2029}); profile class_year 1 -> No (excluded in Jobs default); class_year 2 -> Apply
        let mut years = BTreeSet::new();
        years.insert(2028);
        years.insert(2029);

        let mut facts = clean_facts();
        facts.class_years.value = ClassYears::Grad(years);

        let row = make_row(
            1,
            "Acme",
            "SWE",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(facts),
            100,
        );

        let mut p1 = test_profile();
        p1.class_year = 1;
        let rows1 = [row.clone()];
        let view1 = build_view(&rows1, Tab::Jobs, VerdictFilter::ApplyMaybe, "", &p1, 2027);
        assert_eq!(view1.len(), 0);

        let mut p2 = test_profile();
        p2.class_year = 2;
        let rows2 = [row];
        let view2 = build_view(&rows2, Tab::Jobs, VerdictFilter::ApplyMaybe, "", &p2, 2027);
        assert_eq!(view2.len(), 1);
        assert_eq!(view2[0].judgement.verdict, Verdict::Apply);
    }
}
