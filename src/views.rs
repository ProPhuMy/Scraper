#![allow(dead_code)]

use std::collections::HashMap;

use crate::config::Profile;
use crate::db::{Marks, Row};
use crate::model::Status;
use crate::verdict::{judge_row, llm_hint, Judgement, Verdict};

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

pub fn is_closed(row: &Row) -> bool {
    !row.active || row.status == Status::Gone
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
    pub hint: Option<Judgement>,
}

pub fn llm_rank(vr: &ViewRow) -> u8 {
    if vr.judgement.verdict != Verdict::Maybe {
        return 0;
    }
    match vr.hint.as_ref().map(|h| h.verdict) {
        Some(Verdict::Apply) => 0,
        Some(Verdict::Maybe) => 1,
        Some(Verdict::No) => 2,
        None => 3,
    }
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
                    || row.locations.iter().any(|loc| loc.to_lowercase().contains(&needle))
            }
        })
        .map(|row| {
            let judgement = judge_row(row, profile, season_year);
            let hint = llm_hint(row, profile, season_year);
            ViewRow { row, judgement, hint }
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
                llm_rank(vr),
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

#[derive(Debug, PartialEq, Eq, Hash)]
struct LocRowKey {
    company: String,
    title: String,
    verdict: u8,
    locations: Vec<String>,
}

fn verdict_key(v: Verdict) -> u8 {
    match v {
        Verdict::Apply => 0,
        Verdict::Maybe => 1,
        Verdict::No => 2,
    }
}

fn make_loc_row_key(vr: &ViewRow) -> LocRowKey {
    let mut locs: Vec<String> = vr
        .row
        .locations
        .iter()
        .map(|loc| loc.trim().to_lowercase())
        .collect();
    locs.sort();
    LocRowKey {
        company: vr.row.job.company.trim().to_lowercase(),
        title: vr.row.job.title.trim().to_lowercase(),
        verdict: verdict_key(vr.judgement.verdict),
        locations: locs,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocRow<'a> {
    pub members: Vec<ViewRow<'a>>,
}

impl<'a> LocRow<'a> {
    pub fn newest(&self) -> &ViewRow<'a> {
        &self.members[0]
    }

    pub fn marks(&self) -> Marks {
        Marks {
            starred: self.members.iter().any(|m| m.row.marks.starred),
            hidden: self.members.iter().any(|m| m.row.marks.hidden),
            applied_at: self.members.iter().filter_map(|m| m.row.marks.applied_at).max(),
        }
    }

    pub fn is_new(&self) -> bool {
        self.members.iter().any(|m| m.row.is_new)
    }

    pub fn label(&self) -> String {
        let newest = self.newest();
        let locs = &newest.row.locations;
        let mut s = if locs.is_empty() || locs[0].trim().is_empty() {
            "(no location)".to_string()
        } else {
            let mut base = locs[0].clone();
            if locs.len() > 1 {
                base.push_str(&format!(" +{}", locs.len() - 1));
            }
            base
        };
        if self.members.len() > 1 {
            s.push_str(&format!(" ({} links)", self.members.len()));
        }
        s
    }

    pub fn job_ids(&self) -> Vec<i64> {
        self.members.iter().map(|m| m.row.job.id).collect()
    }

    pub fn is_closed(&self) -> bool {
        self.members.iter().all(|m| is_closed(m.row))
    }
}

pub fn merge_locations<'a>(view: Vec<ViewRow<'a>>) -> Vec<LocRow<'a>> {
    let mut map: HashMap<LocRowKey, usize> = HashMap::new();
    let mut result: Vec<LocRow<'a>> = Vec::new();

    for vr in view {
        let key = make_loc_row_key(&vr);
        if let Some(&idx) = map.get(&key) {
            result[idx].members.push(vr);
        } else {
            map.insert(key, result.len());
            result.push(LocRow {
                members: vec![vr],
            });
        }
    }

    for loc_row in &mut result {
        loc_row.members.sort_by(|a, b| {
            b.row
                .date_posted
                .cmp(&a.row.date_posted)
                .then_with(|| b.row.job.id.cmp(&a.row.job.id))
        });
    }

    result
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct GroupKey {
    company: String,
    title: String,
    verdict: u8,
}

fn make_group_key(loc: &LocRow) -> GroupKey {
    let newest = loc.newest();
    GroupKey {
        company: newest.row.job.company.trim().to_lowercase(),
        title: newest.row.job.title.trim().to_lowercase(),
        verdict: verdict_key(newest.judgement.verdict),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    pub rows: Vec<LocRow<'a>>,
}

pub fn group_view<'a>(locs: Vec<LocRow<'a>>) -> Vec<Group<'a>> {
    let mut map: HashMap<GroupKey, usize> = HashMap::new();
    let mut groups: Vec<Group<'a>> = Vec::new();

    for loc in locs {
        let key = make_group_key(&loc);
        if let Some(&idx) = map.get(&key) {
            groups[idx].rows.push(loc);
        } else {
            map.insert(key, groups.len());
            groups.push(Group { rows: vec![loc] });
        }
    }

    groups
}

pub fn toggle_many(
    current: &[Marks],
    key: MarkKey,
    now: i64,
) -> Result<Vec<Marks>, &'static str> {
    match key {
        MarkKey::Star => {
            let any_starred = current.iter().any(|m| m.starred);
            if any_starred {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: false,
                        hidden: m.hidden,
                        applied_at: m.applied_at,
                    })
                    .collect())
            } else {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: true,
                        hidden: false,
                        applied_at: m.applied_at,
                    })
                    .collect())
            }
        }
        MarkKey::Hide => {
            if current.iter().any(|m| m.applied_at.is_some()) {
                return Err("applied jobs can't be hidden");
            }
            let any_hidden = current.iter().any(|m| m.hidden);
            if any_hidden {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: m.starred,
                        hidden: false,
                        applied_at: m.applied_at,
                    })
                    .collect())
            } else {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: false,
                        hidden: true,
                        applied_at: m.applied_at,
                    })
                    .collect())
            }
        }
        MarkKey::Applied => {
            let any_applied = current.iter().any(|m| m.applied_at.is_some());
            if any_applied {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: m.starred,
                        hidden: m.hidden,
                        applied_at: None,
                    })
                    .collect())
            } else {
                Ok(current
                    .iter()
                    .map(|m| Marks {
                        starred: m.starred,
                        hidden: m.hidden,
                        applied_at: Some(now),
                    })
                    .collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use crate::verdict::judge;
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
            llm_facts: None,
            date_posted,
            active,
            marks,
            is_new: false,
            locations: vec![],
            asks_location: false,
            form_checked: false,
            note: None,
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

    fn make_row_with_locs(
        id: i64,
        company: &str,
        title: &str,
        active: bool,
        marks: Marks,
        status: Status,
        facts: Option<Facts>,
        date_posted: i64,
        locations: Vec<String>,
    ) -> Row {
        let mut r = make_row(id, company, title, active, marks, status, facts, date_posted);
        r.locations = locations;
        r
    }

    #[test]
    fn location_list_order_insensitive() {
        // postings A ["SF","NYC"], B ["NYC","SF"] same company/title/verdict -> 1 LocRow with 2 members; C ["SF"] -> separate LocRow
        let row_a = make_row_with_locs(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec!["SF".to_string(), "NYC".to_string()],
        );
        let row_b = make_row_with_locs(
            2,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            200,
            vec!["NYC".to_string(), "SF".to_string()],
        );
        let row_c = make_row_with_locs(
            3,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            150,
            vec!["SF".to_string()],
        );
        let profile = test_profile();
        let rows = [row_a, row_b, row_c];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        assert_eq!(loc_rows.len(), 2);
        assert_eq!(loc_rows[0].members.len(), 2);
        assert_eq!(loc_rows[1].members.len(), 1);
    }

    #[test]
    fn verdict_splits_groups() {
        // same company+title, one Apply one No -> 2 groups
        let mut no_facts = clean_facts();
        no_facts.citizenship.value = Citizenship::Required;
        let row_apply = make_row(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
        );
        let row_no = make_row(
            2,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(no_facts),
            200,
        );
        let profile = test_profile();
        let rows = [row_apply, row_no];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        let groups = group_view(loc_rows);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_order_and_star_pin() {
        // sorted view [starred L3Harris SLC, Acme, L3Harris SD] -> groups [L3Harris(2 loc rows), Acme]
        let l3_slc = make_row_with_locs(
            1,
            "L3Harris",
            "Intern",
            true,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec!["Salt Lake City, UT".into()],
        );
        let acme = make_row_with_locs(
            2,
            "Acme",
            "Intern",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec!["Remote".into()],
        );
        let l3_sd = make_row_with_locs(
            3,
            "L3Harris",
            "Intern",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            50,
            vec!["San Diego, CA".into()],
        );
        let profile = test_profile();
        let rows = [l3_slc, acme, l3_sd];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        let groups = group_view(loc_rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[0].rows[0].newest().row.job.company, "L3Harris");
        assert_eq!(groups[1].rows.len(), 1);
        assert_eq!(groups[1].rows[0].newest().row.job.company, "Acme");
    }

    #[test]
    fn merged_marks_any_rule() {
        // LocRow of 2 postings, one starred: marks().starred == true; toggle_many(Star) -> both false;
        // none starred -> both true; one applied -> toggle_many(Hide) == Err("applied jobs can't be hidden")
        let row1 = make_row(
            1,
            "Acme",
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
            "Acme",
            "Engineer",
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
        let profile = test_profile();
        let rows = [row1, row2];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        assert_eq!(loc_rows.len(), 1);
        assert_eq!(loc_rows[0].marks().starred, true);

        let now = 1000;
        let marks_in = [rows[0].marks, rows[1].marks];
        let res_star = toggle_many(&marks_in, MarkKey::Star, now).unwrap();
        assert_eq!(
            res_star,
            vec![
                Marks {
                    starred: false,
                    hidden: false,
                    applied_at: None,
                },
                Marks {
                    starred: false,
                    hidden: false,
                    applied_at: None,
                },
            ]
        );

        let marks_unstarred = [
            Marks {
                starred: false,
                hidden: false,
                applied_at: None,
            },
            Marks {
                starred: false,
                hidden: false,
                applied_at: None,
            },
        ];
        let res_star_on = toggle_many(&marks_unstarred, MarkKey::Star, now).unwrap();
        assert_eq!(
            res_star_on,
            vec![
                Marks {
                    starred: true,
                    hidden: false,
                    applied_at: None,
                },
                Marks {
                    starred: true,
                    hidden: false,
                    applied_at: None,
                },
            ]
        );

        let marks_applied = [
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(500),
            },
            Marks {
                starred: false,
                hidden: false,
                applied_at: None,
            },
        ];
        let res_hide = toggle_many(&marks_applied, MarkKey::Hide, now);
        assert_eq!(res_hide, Err("applied jobs can't be hidden"));
    }

    #[test]
    fn labels() {
        // ["Atlanta, GA","Boston, MA","X","Y","Z"] 1 posting -> "Atlanta, GA +4"; 2 postings ["Salt Lake City, UT"] -> "Salt Lake City, UT (2 links)"; [] -> "(no location)"
        let row1 = make_row_with_locs(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            300,
            vec![
                "Atlanta, GA".into(),
                "Boston, MA".into(),
                "X".into(),
                "Y".into(),
                "Z".into(),
            ],
        );
        let row2a = make_row_with_locs(
            2,
            "Beta",
            "Dev",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec!["Salt Lake City, UT".into()],
        );
        let row2b = make_row_with_locs(
            3,
            "Beta",
            "Dev",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            200,
            vec!["Salt Lake City, UT".into()],
        );
        let row3 = make_row_with_locs(
            4,
            "Gamma",
            "QA",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec![],
        );
        let profile = test_profile();
        let rows = [row1, row2a, row2b, row3];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        assert_eq!(loc_rows.len(), 3);
        assert_eq!(loc_rows[0].label(), "Atlanta, GA +4");
        assert_eq!(loc_rows[1].label(), "Salt Lake City, UT (2 links)");
        assert_eq!(loc_rows[2].label(), "(no location)");
    }

    #[test]
    fn newest_member_first() {
        // date_posted 100 and 200 -> newest().row.date_posted == 200
        let row1 = make_row(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
        );
        let row2 = make_row(
            2,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            200,
        );
        let profile = test_profile();
        let rows = [row1, row2];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &profile, 2027);
        let loc_rows = merge_locations(view);
        assert_eq!(loc_rows.len(), 1);
        assert_eq!(loc_rows[0].newest().row.date_posted, 200);
        assert_eq!(loc_rows[0].members[0].row.date_posted, 200);
    }

    #[test]
    fn search_matches_location() {
        // search "rochester" finds a row with location "Rochester, NY"
        let row = make_row_with_locs(
            1,
            "Acme",
            "Engineer",
            true,
            Marks::default(),
            Status::RulesDone,
            Some(clean_facts()),
            100,
            vec!["Rochester, NY".into()],
        );
        let profile = test_profile();
        let rows = [row];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "rochester", &profile, 2027);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].row.job.company, "Acme");
    }

    #[test]
    fn is_closed_rules() {
        let row_inactive = make_row(1, "Acme", "SWE", false, Marks::default(), Status::RulesDone, Some(clean_facts()), 100);
        let row_gone = make_row(2, "Acme", "SWE", true, Marks::default(), Status::Gone, Some(clean_facts()), 100);
        let row_open = make_row(3, "Acme", "SWE", true, Marks::default(), Status::RulesDone, Some(clean_facts()), 100);

        assert_eq!(is_closed(&row_inactive), true);
        assert_eq!(is_closed(&row_gone), true);
        assert_eq!(is_closed(&row_open), false);
    }

    #[test]
    fn closed_only_when_all_members_closed() {
        let row_gone = make_row(1, "Acme", "SWE", true, Marks::default(), Status::Gone, Some(clean_facts()), 100);
        let row_open = make_row(2, "Acme", "SWE", true, Marks::default(), Status::RulesDone, Some(clean_facts()), 100);
        let row_inactive = make_row(3, "Acme", "SWE", false, Marks::default(), Status::RulesDone, Some(clean_facts()), 100);

        let profile = test_profile();
        let j_gone = judge(row_gone.status, row_gone.facts.as_ref(), &profile, 2027);
        let j_open = judge(row_open.status, row_open.facts.as_ref(), &profile, 2027);
        let j_inactive = judge(row_inactive.status, row_inactive.facts.as_ref(), &profile, 2027);

        let loc_mixed = LocRow {
            members: vec![
                ViewRow { row: &row_gone, judgement: j_gone.clone(), hint: None },
                ViewRow { row: &row_open, judgement: j_open, hint: None },
            ],
        };
        assert_eq!(loc_mixed.is_closed(), false);

        let loc_all_closed = LocRow {
            members: vec![
                ViewRow { row: &row_gone, judgement: j_gone, hint: None },
                ViewRow { row: &row_inactive, judgement: j_inactive, hint: None },
            ],
        };
        assert_eq!(loc_all_closed.is_closed(), true);
    }

    #[test]
    fn maybe_sorted_by_llm_hint() {
        let p = test_profile(); // grad year 2030, season 2027

        // 1 apply row
        let row_apply = make_row(0, "ApplyCorp", "SWE", true, Marks::default(), Status::RulesDone, Some(clean_facts()), 1000);

        // 4 maybe rows
        // Apply hint:
        let mut facts_maybe_sp = clean_facts();
        facts_maybe_sp.sponsorship.value = Sponsorship::Unknown;
        let mut llm_sp_offered = Facts::unknown();
        llm_sp_offered.sponsorship = Fact {
            value: Sponsorship::Offered,
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Llm,
                snippet: "sponsorship offered".to_string(),
            }),
        };
        let mut row_hint_apply = make_row(1, "HintApply", "SWE", true, Marks::default(), Status::RulesDone, Some(facts_maybe_sp.clone()), 1000);
        row_hint_apply.llm_facts = Some(llm_sp_offered);

        // No hint:
        let mut facts_maybe_cy = clean_facts();
        facts_maybe_cy.class_years.value = ClassYears::Unknown;
        let mut llm_cy_no = Facts::unknown();
        let mut years = BTreeSet::new();
        years.insert(2027);
        years.insert(2028);
        llm_cy_no.class_years = Fact {
            value: ClassYears::Grad(years),
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Llm,
                snippet: "2027 or 2028".to_string(),
            }),
        };
        let mut row_hint_no = make_row(2, "HintNo", "SWE", true, Marks::default(), Status::RulesDone, Some(facts_maybe_cy), 1000);
        row_hint_no.llm_facts = Some(llm_cy_no);

        // None hint:
        let row_hint_none = make_row(3, "HintNone", "SWE", true, Marks::default(), Status::RulesDone, Some(facts_maybe_sp.clone()), 1000);

        // Maybe hint:
        let mut facts_maybe_two = clean_facts();
        facts_maybe_two.sponsorship.value = Sponsorship::Unknown;
        facts_maybe_two.gpa.value = Gpa::Unknown;
        let mut llm_sp_only = Facts::unknown();
        llm_sp_only.sponsorship = Fact {
            value: Sponsorship::Offered,
            evidence: Some(crate::model::Evidence {
                source: crate::model::Source::Llm,
                snippet: "sponsorship offered".to_string(),
            }),
        };
        let mut row_hint_maybe = make_row(4, "HintMaybe", "SWE", true, Marks::default(), Status::RulesDone, Some(facts_maybe_two), 1000);
        row_hint_maybe.llm_facts = Some(llm_sp_only);

        // Input order: Apply (0), hint Apply (1), hint No (2), hint None (3), hint Maybe (4)
        let rows = [row_apply, row_hint_apply, row_hint_no, row_hint_none, row_hint_maybe];
        let view = build_view(&rows, Tab::Jobs, VerdictFilter::All, "", &p, 2027);

        // Expected sorted order: Apply (0), hint Apply (1), hint Maybe (4), hint No (2), hint None (3)
        let ids: Vec<i64> = view.iter().map(|vr| vr.row.job.id).collect();
        assert_eq!(ids, vec![0, 1, 4, 2, 3]);
    }
}
