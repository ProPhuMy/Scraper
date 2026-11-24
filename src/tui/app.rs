use std::collections::HashSet;
use crate::config::{Config, Profile};
use crate::db::{now_unix, Marks, Row};
use crate::verdict::Verdict;
use crate::views::{
    build_view, group_view, merge_locations, toggle_many, LocRow, MarkKey, Tab,
    VerdictFilter, ViewRow,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SaveMarks(Vec<(i64, Marks)>),
    OpenUrl(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Plain(usize),
    Header(usize),
    Child(usize, usize),
}

pub fn group_key(company: &str, title: &str, verdict: Verdict) -> String {
    let v_str = match verdict {
        Verdict::Apply => "apply",
        Verdict::Maybe => "maybe",
        Verdict::No => "no",
    };
    format!(
        "{}\u{1f}{}\u{1f}{}",
        company.trim().to_lowercase(),
        title.trim().to_lowercase(),
        v_str
    )
}

pub fn group_key_from_loc(loc: &LocRow) -> String {
    let newest = loc.newest();
    group_key(
        &newest.row.job.company,
        &newest.row.job.title,
        newest.judgement.verdict,
    )
}

pub struct App {
    pub rows: Vec<Row>,
    pub tab: Tab,
    pub filter: VerdictFilter,
    pub search: String,
    pub searching: bool,
    pub class_year: u8,
    pub selected: usize,
    pub status_msg: Option<String>,
    pub show_help: bool,
    pub quit: bool,
    pub season_year: u16,
    pub profile: Profile,
    pub expanded: HashSet<String>,
    pub lines: Vec<Line>,
}

impl App {
    pub fn new(rows: Vec<Row>, cfg: &Config) -> App {
        let mut app = App {
            rows,
            tab: Tab::Jobs,
            filter: VerdictFilter::ApplyMaybe,
            search: String::new(),
            searching: false,
            class_year: cfg.profile.class_year,
            selected: 0,
            status_msg: None,
            show_help: false,
            quit: false,
            season_year: cfg.season_year,
            profile: cfg.profile.clone(),
            expanded: HashSet::new(),
            lines: Vec::new(),
        };
        app.rebuild_lines();
        app.clamp_selected();
        app
    }

    pub fn visible_rows<'a>(&'a self) -> Vec<ViewRow<'a>> {
        let mut profile = self.profile.clone();
        profile.class_year = self.class_year;
        profile.expected_graduation = None;
        build_view(
            &self.rows,
            self.tab,
            self.filter,
            &self.search,
            &profile,
            self.season_year,
        )
    }

    pub fn lines(&self) -> Vec<Line> {
        let view = self.visible_rows();
        let loc_rows = merge_locations(view);
        if self.tab == Tab::Jobs {
            let groups = group_view(loc_rows);
            let mut lines = Vec::new();
            for (g_idx, group) in groups.iter().enumerate() {
                if group.rows.len() == 1 {
                    lines.push(Line::Plain(g_idx));
                } else {
                    lines.push(Line::Header(g_idx));
                    let key = group_key_from_loc(&group.rows[0]);
                    if self.expanded.contains(&key) {
                        for loc_idx in 0..group.rows.len() {
                            lines.push(Line::Child(g_idx, loc_idx));
                        }
                    }
                }
            }
            lines
        } else {
            (0..loc_rows.len()).map(Line::Plain).collect()
        }
    }

    pub fn rebuild_lines(&mut self) {
        self.lines = self.lines();
    }

    pub fn clamp_selected(&mut self) {
        let len = self.lines.len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }

        self.rebuild_lines();
        self.clamp_selected();

        // Ctrl-C arrives as a key event in raw mode: quit
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return None;
        }

        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return None;
        }

        if self.searching {
            match key.code {
                KeyCode::Char(c) => {
                    self.search.push(c);
                    self.rebuild_lines();
                    self.clamp_selected();
                }
                KeyCode::Backspace => {
                    self.search.pop();
                    self.rebuild_lines();
                    self.clamp_selected();
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Esc => {
                    self.search.clear();
                    self.searching = false;
                    self.rebuild_lines();
                    self.clamp_selected();
                }
                _ => {}
            }
            return None;
        }

        // Normal mode
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                None
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                None
            }
            KeyCode::Char('/') => {
                self.searching = true;
                None
            }
            KeyCode::Esc => {
                if !self.search.is_empty() {
                    self.search.clear();
                    self.rebuild_lines();
                    self.clamp_selected();
                }
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.lines.len();
                if len > 0 && self.selected + 1 < len {
                    self.selected += 1;
                }
                None
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                None
            }
            KeyCode::PageDown => {
                let len = self.lines.len();
                if len > 0 {
                    self.selected = (self.selected + 10).min(len - 1);
                }
                None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                None
            }
            KeyCode::End | KeyCode::Char('G') => {
                let len = self.lines.len();
                self.selected = len.saturating_sub(1);
                None
            }
            KeyCode::Tab => {
                self.tab = match self.tab {
                    Tab::Jobs => Tab::Starred,
                    Tab::Starred => Tab::Applied,
                    Tab::Applied => Tab::Hidden,
                    Tab::Hidden => Tab::Jobs,
                };
                self.status_msg = None;
                self.rebuild_lines();
                self.clamp_selected();
                None
            }
            KeyCode::BackTab => {
                self.tab = match self.tab {
                    Tab::Jobs => Tab::Hidden,
                    Tab::Hidden => Tab::Applied,
                    Tab::Applied => Tab::Starred,
                    Tab::Starred => Tab::Jobs,
                };
                self.status_msg = None;
                self.rebuild_lines();
                self.clamp_selected();
                None
            }
            KeyCode::Char('1') => {
                if self.tab == Tab::Jobs {
                    self.filter = VerdictFilter::Apply;
                    self.status_msg = None;
                    self.rebuild_lines();
                    self.clamp_selected();
                } else {
                    self.status_msg = Some("verdict filter only applies to Jobs tab".to_string());
                }
                None
            }
            KeyCode::Char('2') => {
                if self.tab == Tab::Jobs {
                    self.filter = VerdictFilter::Maybe;
                    self.status_msg = None;
                    self.rebuild_lines();
                    self.clamp_selected();
                } else {
                    self.status_msg = Some("verdict filter only applies to Jobs tab".to_string());
                }
                None
            }
            KeyCode::Char('3') => {
                if self.tab == Tab::Jobs {
                    self.filter = VerdictFilter::No;
                    self.status_msg = None;
                    self.rebuild_lines();
                    self.clamp_selected();
                } else {
                    self.status_msg = Some("verdict filter only applies to Jobs tab".to_string());
                }
                None
            }
            KeyCode::Char('0') => {
                if self.tab == Tab::Jobs {
                    self.filter = VerdictFilter::All;
                    self.status_msg = None;
                    self.rebuild_lines();
                    self.clamp_selected();
                } else {
                    self.status_msg = Some("verdict filter only applies to Jobs tab".to_string());
                }
                None
            }
            KeyCode::Char('y') => {
                self.class_year = if self.class_year >= 4 { 1 } else { self.class_year + 1 };
                self.rebuild_lines();
                self.clamp_selected();
                None
            }
            KeyCode::Enter => {
                if self.tab == Tab::Jobs {
                    let view = self.visible_rows();
                    let loc_rows = merge_locations(view);
                    let groups = group_view(loc_rows);
                    if let Some(&line) = self.lines.get(self.selected) {
                        match line {
                            Line::Header(g_idx) => {
                                let key = group_key_from_loc(&groups[g_idx].rows[0]);
                                if self.expanded.contains(&key) {
                                    self.expanded.remove(&key);
                                } else {
                                    self.expanded.insert(key);
                                }
                                self.rebuild_lines();
                                if let Some(pos) = self.lines.iter().position(|l| matches!(l, Line::Header(g) if *g == g_idx)) {
                                    self.selected = pos;
                                }
                                self.clamp_selected();
                            }
                            Line::Child(g_idx, _) => {
                                let key = group_key_from_loc(&groups[g_idx].rows[0]);
                                self.expanded.remove(&key);
                                self.rebuild_lines();
                                if let Some(pos) = self.lines.iter().position(|l| matches!(l, Line::Header(g) if *g == g_idx)) {
                                    self.selected = pos;
                                }
                                self.clamp_selected();
                            }
                            Line::Plain(_) => {}
                        }
                    }
                }
                None
            }
            KeyCode::Right => {
                if self.tab == Tab::Jobs {
                    let view = self.visible_rows();
                    let loc_rows = merge_locations(view);
                    let groups = group_view(loc_rows);
                    if let Some(&Line::Header(g_idx)) = self.lines.get(self.selected) {
                        let key = group_key_from_loc(&groups[g_idx].rows[0]);
                        if !self.expanded.contains(&key) {
                            self.expanded.insert(key);
                            self.rebuild_lines();
                            if let Some(pos) = self.lines.iter().position(|l| matches!(l, Line::Header(g) if *g == g_idx)) {
                                self.selected = pos;
                            }
                            self.clamp_selected();
                        }
                    }
                }
                None
            }
            KeyCode::Left => {
                if self.tab == Tab::Jobs {
                    let view = self.visible_rows();
                    let loc_rows = merge_locations(view);
                    let groups = group_view(loc_rows);
                    if let Some(&line) = self.lines.get(self.selected) {
                        match line {
                            Line::Header(g_idx) => {
                                let key = group_key_from_loc(&groups[g_idx].rows[0]);
                                if self.expanded.contains(&key) {
                                    self.expanded.remove(&key);
                                    self.rebuild_lines();
                                    if let Some(pos) = self.lines.iter().position(|l| matches!(l, Line::Header(g) if *g == g_idx)) {
                                        self.selected = pos;
                                    }
                                    self.clamp_selected();
                                }
                            }
                            Line::Child(g_idx, _) => {
                                let key = group_key_from_loc(&groups[g_idx].rows[0]);
                                self.expanded.remove(&key);
                                self.rebuild_lines();
                                if let Some(pos) = self.lines.iter().position(|l| matches!(l, Line::Header(g) if *g == g_idx)) {
                                    self.selected = pos;
                                }
                                self.clamp_selected();
                            }
                            Line::Plain(_) => {}
                        }
                    }
                }
                None
            }
            KeyCode::Char('s') => self.toggle_mark(MarkKey::Star),
            KeyCode::Char('h') => self.toggle_mark(MarkKey::Hide),
            KeyCode::Char('a') => self.toggle_mark(MarkKey::Applied),
            KeyCode::Char('o') => {
                let view = self.visible_rows();
                let loc_rows = merge_locations(view);
                match self.lines.get(self.selected) {
                    Some(Line::Header(_)) => {
                        self.status_msg = Some("press Enter to pick a location".to_string());
                        None
                    }
                    Some(Line::Child(g_idx, loc_idx)) => {
                        let groups = group_view(loc_rows);
                        let url = groups.get(*g_idx)?.rows.get(*loc_idx)?.newest().row.job.url.clone();
                        Some(Action::OpenUrl(url))
                    }
                    Some(Line::Plain(idx)) => {
                        let url = if self.tab == Tab::Jobs {
                            let groups = group_view(loc_rows);
                            groups.get(*idx)?.rows.get(0)?.newest().row.job.url.clone()
                        } else {
                            loc_rows.get(*idx)?.newest().row.job.url.clone()
                        };
                        Some(Action::OpenUrl(url))
                    }
                    None => None,
                }
            }
            _ => None,
        }
    }

    fn toggle_mark(&mut self, key: MarkKey) -> Option<Action> {
        let selected_line = *self.lines.get(self.selected)?;

        let (current_marks, job_ids) = match selected_line {
            Line::Header(_) => {
                self.status_msg = Some("press Enter to pick a location".to_string());
                return None;
            }
            Line::Child(g_idx, loc_idx) => {
                let view = self.visible_rows();
                let loc_rows = merge_locations(view);
                let groups = group_view(loc_rows);
                let loc_row = groups.get(g_idx)?.rows.get(loc_idx)?;
                (
                    loc_row.members.iter().map(|m| m.row.marks).collect::<Vec<_>>(),
                    loc_row.job_ids(),
                )
            }
            Line::Plain(idx) => {
                let view = self.visible_rows();
                let loc_rows = merge_locations(view);
                if self.tab == Tab::Jobs {
                    let groups = group_view(loc_rows);
                    let loc_row = groups.get(idx)?.rows.get(0)?;
                    (
                        loc_row.members.iter().map(|m| m.row.marks).collect::<Vec<_>>(),
                        loc_row.job_ids(),
                    )
                } else {
                    let loc_row = loc_rows.get(idx)?;
                    (
                        loc_row.members.iter().map(|m| m.row.marks).collect::<Vec<_>>(),
                        loc_row.job_ids(),
                    )
                }
            }
        };

        let now = now_unix();
        match toggle_many(&current_marks, key, now) {
            Ok(new_marks_vec) => {
                self.status_msg = None;
                let mut save_marks = Vec::with_capacity(job_ids.len());
                for (job_id, new_marks) in job_ids.into_iter().zip(new_marks_vec) {
                    save_marks.push((job_id, new_marks));
                    if let Some(row) = self.rows.iter_mut().find(|r| r.job.id == job_id) {
                        row.marks = new_marks;
                    }
                }
                self.rebuild_lines();
                self.clamp_selected();
                Some(Action::SaveMarks(save_marks))
            }
            Err(err) => {
                self.status_msg = Some(err.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use crate::model::{
        Citizenship, ClassYears, Experience, Fact, Facts, Gpa, Job, Provider, Sponsorship, Status,
    };

    fn test_config() -> Config {
        Config {
            season_year: 2027,
            profile: Profile {
                class_year: 1,
                expected_graduation: None,
                gpa: None,
                needs_sponsorship: true,
                has_prior_internship: false,
            },
            llm: None,
            scan: crate::config::ScanConfig::default(),
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

    fn make_row(id: i64, company: &str, title: &str, active: bool, marks: Marks) -> Row {
        Row {
            job: Job {
                id,
                simplify_id: format!("job_{id}"),
                company: company.to_string(),
                title: title.to_string(),
                url: format!("https://example.com/job/{id}"),
                provider: Provider::Other,
            },
            status: Status::RulesDone,
            facts: Some(clean_facts()),
            date_posted: 1000,
            active,
            marks,
            is_new: false,
            locations: vec![],
            asks_location: false,
        }
    }

    #[test]
    fn star_key_emits_save() {
        let row = make_row(1, "Acme", "SWE", true, Marks::default());
        let cfg = test_config();
        let mut app = App::new(vec![row], &cfg);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        let action = app.on_key(key);
        assert_eq!(
            action,
            Some(Action::SaveMarks(vec![(
                1,
                Marks {
                    starred: true,
                    hidden: false,
                    applied_at: None,
                }
            )]))
        );
    }

    #[test]
    fn hide_applied_shows_message() {
        let row = make_row(
            1,
            "Acme",
            "SWE",
            true,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(12345),
            },
        );
        let cfg = test_config();
        let mut app = App::new(vec![row], &cfg);
        app.tab = Tab::Applied;
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty());
        let action = app.on_key(key);
        assert!(action.is_none());
        assert_eq!(
            app.status_msg.as_deref(),
            Some("applied jobs can't be hidden")
        );
    }

    #[test]
    fn selection_clamps_after_hide() {
        let r1 = make_row(1, "Acme", "SWE 1", true, Marks::default());
        let r2 = make_row(2, "Acme", "SWE 2", true, Marks::default());
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2], &cfg);
        app.selected = 1;
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty());
        let _ = app.on_key(key);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn verdict_keys_only_on_jobs_tab() {
        let r1 = make_row(
            1,
            "Acme",
            "SWE 1",
            true,
            Marks {
                starred: true,
                hidden: false,
                applied_at: None,
            },
        );
        let cfg = test_config();
        let mut app = App::new(vec![r1], &cfg);
        app.tab = Tab::Starred;
        let orig_filter = app.filter;
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::empty());
        let _ = app.on_key(key);
        assert_eq!(app.filter, orig_filter);
        assert!(app.status_msg.is_some());
    }

    #[test]
    fn search_mode_typing() {
        let r1 = make_row(1, "Neuralink", "SWE", true, Marks::default());
        let cfg = test_config();
        let mut app = App::new(vec![r1], &cfg);
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        assert!(app.searching);
        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()));
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()));
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::empty()));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(app.search, "neu");
        assert!(!app.searching);
    }

    #[test]
    fn year_cycles() {
        let r1 = make_row(1, "Acme", "SWE", true, Marks::default());
        let mut cfg = test_config();
        cfg.profile.class_year = 4;
        let mut app = App::new(vec![r1], &cfg);
        assert_eq!(app.class_year, 4);
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty());
        let _ = app.on_key(key);
        assert_eq!(app.class_year, 1);
    }

    fn make_row_with_locs(
        id: i64,
        company: &str,
        title: &str,
        active: bool,
        marks: Marks,
        date_posted: i64,
        locations: Vec<String>,
    ) -> Row {
        let mut r = make_row(id, company, title, active, marks);
        r.date_posted = date_posted;
        r.locations = locations;
        r
    }

    #[test]
    fn header_collapsed_by_default() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["NYC".into()]);
        let cfg = test_config();
        let app = App::new(vec![r1, r2], &cfg);
        assert_eq!(app.lines.len(), 1);
        assert!(matches!(app.lines[0], Line::Header(_)));
    }

    #[test]
    fn enter_expands_and_collapses() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["NYC".into()]);
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2], &cfg);
        assert_eq!(app.lines.len(), 1);

        // Enter on header expands
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(app.lines.len(), 3);
        assert_eq!(app.selected, 0);

        // Select child
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        assert_eq!(app.selected, 1);

        // Enter on child collapses parent and moves selection to header
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(app.lines.len(), 1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn marks_on_header_blocked() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["NYC".into()]);
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2], &cfg);
        let action = app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()));
        assert!(action.is_none());
        assert_eq!(app.status_msg.as_deref(), Some("press Enter to pick a location"));
    }

    #[test]
    fn mark_child_saves_all_links() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["SF".into()]);
        let r3 = make_row_with_locs(3, "Acme", "SWE", true, Marks::default(), 150, vec!["NYC".into()]);
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2, r3], &cfg);
        // Expand group
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        // Move to first child (SF, which has 2 postings: id 2 and 1)
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        let action = app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()));
        assert_eq!(
            action,
            Some(Action::SaveMarks(vec![
                (
                    2,
                    Marks {
                        starred: true,
                        hidden: false,
                        applied_at: None,
                    }
                ),
                (
                    1,
                    Marks {
                        starred: true,
                        hidden: false,
                        applied_at: None,
                    }
                ),
            ]))
        );
    }

    #[test]
    fn group_shrinks_to_plain_after_hide() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["NYC".into()]);
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2], &cfg);
        // Expand
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(app.lines.len(), 3);
        // Select child 1 (index 1)
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        assert_eq!(app.selected, 1);
        // Hide it
        let action = app.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty()));
        assert!(action.is_some());
        // Remaining group has 1 location row, so it becomes Plain
        assert_eq!(app.lines.len(), 1);
        assert!(matches!(app.lines[0], Line::Plain(_)));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn open_uses_newest_link() {
        let r1 = make_row_with_locs(1, "Acme", "SWE", true, Marks::default(), 100, vec!["SF".into()]);
        let r2 = make_row_with_locs(2, "Acme", "SWE", true, Marks::default(), 200, vec!["SF".into()]);
        let r3 = make_row_with_locs(3, "Acme", "SWE", true, Marks::default(), 150, vec!["NYC".into()]);
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2, r3], &cfg);
        // Expand
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        // Select child (SF, newest is id 2)
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        let action = app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::empty()));
        assert_eq!(action, Some(Action::OpenUrl("https://example.com/job/2".into())));
    }
}
