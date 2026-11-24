use crate::model::{Citizenship, ClassYears, Experience, Gpa, Source, Sponsorship};
use crate::text::truncate_chars;
use crate::tui::app::App;
use crate::verdict::Verdict;
use crate::views::{tab_count, Tab, VerdictFilter, ViewRow};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row as TableRow, Table, TableState};
use ratatui::Frame;

pub fn format_unix_date(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 14241) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 2 || area.height < 2 {
        return;
    }

    let show_detail = area.height >= 20;

    let chunks = if show_detail {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(5),    // Table
                Constraint::Length(9), // Detail panel
                Constraint::Length(1), // Footer
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(0),    // Table
                Constraint::Length(1), // Footer
            ])
            .split(area)
    };

    render_header(frame, chunks[0], app);

    let visible = app.visible_rows();
    if visible.is_empty() {
        render_empty_table(frame, chunks[1], app);
    } else {
        render_table(frame, chunks[1], app, &visible);
    }

    if show_detail {
        render_detail(frame, chunks[2], app, &visible);
        render_footer(frame, chunks[3], app);
    } else {
        render_footer(frame, chunks[2], app);
    }

    if app.show_help {
        render_help_overlay(frame, area);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut profile = app.profile.clone();
    profile.class_year = app.class_year;
    profile.expected_graduation = None;

    let jobs_count = tab_count(&app.rows, Tab::Jobs, app.filter, &app.search, &profile, app.season_year);
    let starred_count = tab_count(&app.rows, Tab::Starred, app.filter, &app.search, &profile, app.season_year);
    let applied_count = tab_count(&app.rows, Tab::Applied, app.filter, &app.search, &profile, app.season_year);
    let hidden_count = tab_count(&app.rows, Tab::Hidden, app.filter, &app.search, &profile, app.season_year);

    let tab_style = |t: Tab| {
        if app.tab == t {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(Color::Gray)
        }
    };

    let grad_year = profile.grad_year(app.season_year);

    let filter_str = match app.filter {
        VerdictFilter::ApplyMaybe => "[apply+maybe]",
        VerdictFilter::Apply => "[apply]",
        VerdictFilter::Maybe => "[maybe]",
        VerdictFilter::No => "[no]",
        VerdictFilter::All => "[all]",
    };

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("Jobs ({jobs_count})"), tab_style(Tab::Jobs)),
        Span::raw("  "),
        Span::styled(format!("Starred ({starred_count})"), tab_style(Tab::Starred)),
        Span::raw("  "),
        Span::styled(format!("Applied ({applied_count})"), tab_style(Tab::Applied)),
        Span::raw("  "),
        Span::styled(format!("Hidden ({hidden_count})"), tab_style(Tab::Hidden)),
        Span::raw("     "),
        Span::styled(format!("year {} · grad {}", app.class_year, grad_year), Style::default().fg(Color::DarkGray)),
        Span::raw("   "),
        Span::styled(filter_str, Style::default().fg(Color::Yellow)),
    ];

    if app.searching {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("/{}_", app.search), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
    } else if !app.search.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("/{}", app.search), Style::default().fg(Color::Green)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_empty_table(frame: &mut Frame, area: Rect, app: &App) {
    let msg = if !app.search.is_empty() {
        format!("no matches for \"{}\"", app.search)
    } else {
        "nothing here".to_string()
    };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(msg)
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn render_table(frame: &mut Frame, area: Rect, app: &App, visible: &[ViewRow]) {
    let header_cells = ["MARK", "VERDICT", "YEAR", "COMPANY", "TITLE", "REASON"]
        .into_iter()
        .map(|h| Span::styled(h, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
    let header = TableRow::new(header_cells).bottom_margin(0);

    let rows: Vec<TableRow> = visible
        .iter()
        .map(|vr| {
            let mut mark_str = String::new();
            if vr.row.marks.starred {
                mark_str.push('★');
            }
            if vr.row.marks.applied_at.is_some() {
                mark_str.push('✓');
            }
            if vr.row.is_new {
                mark_str.push('N');
            }

            let (verdict_str, verdict_color) = match vr.judgement.verdict {
                Verdict::Apply => ("apply", Color::Green),
                Verdict::Maybe => ("maybe", Color::Yellow),
                Verdict::No => ("no", Color::Red),
            };

            let year_str = if vr.judgement.year_confirmed {
                "yes"
            } else {
                "year?"
            };

            let mark_span = Span::styled(mark_str, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD));
            let verdict_span = Span::styled(verdict_str, Style::default().fg(verdict_color));
            let year_span = Span::styled(year_str, Style::default().fg(Color::White));
            let company_span = Span::styled(truncate_chars(&vr.row.job.company, 20), Style::default().fg(Color::White));
            let title_span = Span::styled(truncate_chars(&vr.row.job.title, 35), Style::default().fg(Color::White));
            let reason_span = Span::styled(truncate_chars(&vr.judgement.reason, 60), Style::default().fg(Color::DarkGray));

            TableRow::new(vec![
                mark_span,
                verdict_span,
                year_span,
                company_span,
                title_span,
                reason_span,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),       // MARK
        Constraint::Length(8),       // VERDICT
        Constraint::Length(6),       // YEAR
        Constraint::Length(22),      // COMPANY
        Constraint::Length(37),      // TITLE
        Constraint::Min(20),         // REASON
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)))
        .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App, visible: &[ViewRow]) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let Some(vr) = visible.get(app.selected) else {
        frame.render_widget(Paragraph::new("").block(block), area);
        return;
    };

    let posted_str = format_unix_date(vr.row.date_posted);
    let provider_str = vr.row.job.provider.as_str();
    let status_str = vr.row.status.as_str();

    let title_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("{} — {}", vr.row.job.company, vr.row.job.title), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled(format!("posted {posted_str} · {provider_str} · {status_str}"), Style::default().fg(Color::DarkGray)),
    ]);

    let format_fact_line = |name: &'static str, val: &str, ev: Option<(&str, &str)>| -> Line<'static> {
        let mut spans = vec![
            Span::raw(" "),
            Span::styled(format!("{:<13}", name), Style::default().fg(Color::Cyan)),
            Span::styled(format!("{:<14}", val), Style::default().fg(Color::White)),
        ];
        if let Some((src, snippet)) = ev {
            spans.push(Span::styled(format!("{}: \"{}\"", src, truncate_chars(snippet, 80)), Style::default().fg(Color::DarkGray)));
        }
        Line::from(spans)
    };

    let mut lines = vec![title_line];

    let source_str = |s: Source| match s {
        Source::Form => "form",
        Source::Description => "description",
        Source::Title => "title",
        Source::Llm => "llm",
    };

    if let Some(ref f) = vr.row.facts {
        // sponsorship
        let sp_val = match f.sponsorship.value {
            Sponsorship::Offered => "offered",
            Sponsorship::NotOffered => "not_offered",
            Sponsorship::Asked => "asked",
            Sponsorship::NotMentioned => "not_mentioned",
            Sponsorship::Unknown => "unknown",
        };
        let sp_ev = f.sponsorship.evidence.as_ref().map(|e| (source_str(e.source), e.snippet.as_str()));
        lines.push(format_fact_line("sponsorship", sp_val, sp_ev));

        // citizenship
        let cit_val = match f.citizenship.value {
            Citizenship::Required => "required",
            Citizenship::NotMentioned => "not_mentioned",
            Citizenship::Unknown => "unknown",
        };
        let cit_ev = f.citizenship.evidence.as_ref().map(|e| (source_str(e.source), e.snippet.as_str()));
        lines.push(format_fact_line("citizenship", cit_val, cit_ev));

        // gpa
        let gpa_val = match f.gpa.value {
            Gpa::Min(m) => format!("min {m}"),
            Gpa::Asked => "asked".to_string(),
            Gpa::NotMentioned => "not_mentioned".to_string(),
            Gpa::Unknown => "unknown".to_string(),
        };
        let gpa_ev = f.gpa.evidence.as_ref().map(|e| (source_str(e.source), e.snippet.as_str()));
        lines.push(format_fact_line("gpa", &gpa_val, gpa_ev));

        // experience
        let exp_val = match f.experience.value {
            Experience::Required => "required",
            Experience::Preferred => "preferred",
            Experience::NotMentioned => "not_mentioned",
            Experience::Unknown => "unknown",
        };
        let exp_ev = f.experience.evidence.as_ref().map(|e| (source_str(e.source), e.snippet.as_str()));
        lines.push(format_fact_line("experience", exp_val, exp_ev));

        // class years
        let cy_val = match &f.class_years.value {
            ClassYears::Grad(set) => {
                let years: Vec<String> = set.iter().map(|y| y.to_string()).collect();
                format!("grad({})", years.join(","))
            }
            ClassYears::Any => "any".to_string(),
            ClassYears::Unknown => "unknown".to_string(),
        };
        let cy_ev = f.class_years.evidence.as_ref().map(|e| (source_str(e.source), e.snippet.as_str()));
        lines.push(format_fact_line("class years", &cy_val, cy_ev));
    } else {
        lines.push(Line::from(vec![Span::raw(" (no facts scanned)")]));
    }

    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(vr.row.job.url.clone(), Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED)),
    ]));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = if let Some(ref msg) = app.status_msg {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(msg, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(vec![
            Span::raw(" "),
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::raw(" move  "),
            Span::styled("/", Style::default().fg(Color::Cyan)),
            Span::raw(" search  "),
            Span::styled("1", Style::default().fg(Color::Cyan)),
            Span::raw(" apply "),
            Span::styled("2", Style::default().fg(Color::Cyan)),
            Span::raw(" maybe "),
            Span::styled("3", Style::default().fg(Color::Cyan)),
            Span::raw(" no "),
            Span::styled("0", Style::default().fg(Color::Cyan)),
            Span::raw(" all  "),
            Span::styled("y", Style::default().fg(Color::Cyan)),
            Span::raw(" year  "),
            Span::styled("s", Style::default().fg(Color::Cyan)),
            Span::raw(" star  "),
            Span::styled("h", Style::default().fg(Color::Cyan)),
            Span::raw(" hide  "),
            Span::styled("a", Style::default().fg(Color::Cyan)),
            Span::raw(" applied  "),
            Span::styled("o", Style::default().fg(Color::Cyan)),
            Span::raw(" open  "),
            Span::styled("tab", Style::default().fg(Color::Cyan)),
            Span::raw(" next  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quit  "),
            Span::styled("?", Style::default().fg(Color::Cyan)),
            Span::raw(" help"),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let width = 60.min(area.width.saturating_sub(4));
    let height = 18.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let text = vec![
        Line::from(Span::styled("Navigation & Filtering:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  ↑ / ↓, j / k       Move selection up / down"),
        Line::from("  PgUp / PgDn        Page up / down"),
        Line::from("  g / G              Jump to top / bottom"),
        Line::from("  Tab / Shift+Tab    Cycle between tabs"),
        Line::from("  1, 2, 3, 0         Filter: Apply, Maybe, No, All (Jobs tab)"),
        Line::from("  /                  Search company / title (Esc clears)"),
        Line::from("  y                  Cycle graduation year (1 → 4 → 1)"),
        Line::from(""),
        Line::from(Span::styled("Job Actions:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  s                  Toggle Starred mark"),
        Line::from("  h                  Toggle Hidden mark"),
        Line::from("  a                  Toggle Applied mark"),
        Line::from("  o                  Open posting URL in browser"),
        Line::from(""),
        Line::from("  ?                  Toggle this help overlay"),
        Line::from("  q / Ctrl-C         Quit scraper browse"),
    ];

    let block = Block::default()
        .title(" Help (press ? or Esc to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Profile, ScanConfig};
    use crate::db::{Marks, Row};
    use crate::model::{
        Citizenship, ClassYears, Experience, Fact, Facts, Gpa, Job, Provider, Sponsorship, Status,
    };
    use crate::tui::app::App;
    use crate::views::Tab;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
            scan: ScanConfig::default(),
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
        }
    }

    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let mut s = String::new();
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                s.push_str(buffer[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_tabs_and_rows() {
        let r1 = make_row(1, "AcmeCorp", "SWE", true, Marks::default());
        let cfg = test_config();
        let app = App::new(vec![r1], &cfg);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buffer_str = buffer_to_string(terminal.backend().buffer());
        assert!(buffer_str.contains("Jobs ("));
        assert!(buffer_str.contains("Starred ("));
        assert!(buffer_str.contains("AcmeCorp"));
        assert!(buffer_str.contains("apply"));
    }

    #[test]
    fn renders_empty_tab_message() {
        let r1 = make_row(1, "AcmeCorp", "SWE", true, Marks::default());
        let cfg = test_config();
        let mut app = App::new(vec![r1], &cfg);
        app.tab = Tab::Applied;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buffer_str = buffer_to_string(terminal.backend().buffer());
        assert!(buffer_str.contains("nothing here"));
    }

    #[test]
    fn renders_tiny_terminal() {
        let r1 = make_row(1, "AcmeCorp", "SWE", true, Marks::default());
        let cfg = test_config();
        let app = App::new(vec![r1], &cfg);
        let backend = TestBackend::new(30, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buffer_str = buffer_to_string(terminal.backend().buffer());
        assert!(!buffer_str.contains("posted"));
        assert!(!buffer_str.contains("sponsorship"));
    }
}
