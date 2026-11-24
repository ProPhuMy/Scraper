use crate::config::{Config, Profile};
use crate::db::{now_unix, Marks, Row};
use crate::views::{build_view, toggle, MarkKey, Tab, VerdictFilter, ViewRow};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SaveMarks { job_id: i64, marks: Marks },
    OpenUrl(String),
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
        };
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

    pub fn clamp_selected(&mut self) {
        let len = self.visible_rows().len();
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
                    self.clamp_selected();
                }
                KeyCode::Backspace => {
                    self.search.pop();
                    self.clamp_selected();
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Esc => {
                    if self.search.is_empty() {
                        self.searching = false;
                    } else {
                        self.search.clear();
                        self.searching = false;
                    }
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
                    self.clamp_selected();
                }
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_rows().len();
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
                let len = self.visible_rows().len();
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
                let len = self.visible_rows().len();
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
                self.clamp_selected();
                None
            }
            KeyCode::Char('1') => {
                if self.tab == Tab::Jobs {
                    self.filter = VerdictFilter::Apply;
                    self.status_msg = None;
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
                    self.clamp_selected();
                } else {
                    self.status_msg = Some("verdict filter only applies to Jobs tab".to_string());
                }
                None
            }
            KeyCode::Char('y') => {
                self.class_year = if self.class_year >= 4 { 1 } else { self.class_year + 1 };
                self.clamp_selected();
                None
            }
            KeyCode::Char('s') => self.toggle_mark(MarkKey::Star),
            KeyCode::Char('h') => self.toggle_mark(MarkKey::Hide),
            KeyCode::Char('a') => self.toggle_mark(MarkKey::Applied),
            KeyCode::Char('o') => {
                let visible = self.visible_rows();
                if let Some(vr) = visible.get(self.selected) {
                    Some(Action::OpenUrl(vr.row.job.url.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn toggle_mark(&mut self, key: MarkKey) -> Option<Action> {
        let visible = self.visible_rows();
        let vr = visible.get(self.selected)?;
        let job_id = vr.row.job.id;
        let old_marks = vr.row.marks;
        let now = now_unix();

        match toggle(old_marks, key, now) {
            Ok(new_marks) => {
                self.status_msg = None;
                if let Some(row) = self.rows.iter_mut().find(|r| r.job.id == job_id) {
                    row.marks = new_marks;
                }
                self.clamp_selected();
                Some(Action::SaveMarks {
                    job_id,
                    marks: new_marks,
                })
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
            Some(Action::SaveMarks {
                job_id: 1,
                marks: Marks {
                    starred: true,
                    hidden: false,
                    applied_at: None,
                }
            })
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
}
