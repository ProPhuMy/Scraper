use crate::model::{Citizenship, ClassYears, Experience, Gpa, Source, Sponsorship};
use crate::text::truncate_chars;
use crate::tui::app::{App, Line as DisplayLine};
use crate::verdict::Verdict;
use crate::views::{self, tab_count, LocRow, Tab, VerdictFilter};
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

    let lines = app.lines();
    if lines.is_empty() {
        render_empty_table(frame, chunks[1], app);
    } else {
        render_table(frame, chunks[1], app, &lines);
    }

    if show_detail {
        render_detail(frame, chunks[2], app, &lines);
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

fn format_mark_cell(starred: bool, applied: bool, is_new: bool, closed: bool) -> Line<'static> {
    let mut spans = Vec::new();
    let mut prefix = String::new();
    if starred {
        prefix.push('★');
    }
    if applied {
        prefix.push('✓');
    }
    if is_new {
        prefix.push('N');
    }
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)));
    }
    if closed {
        spans.push(Span::styled("✗", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)));
    }
    Line::from(spans)
}

/// Vim-style scrolloff: keep up to 5 rows visible above and below the cursor,
/// moving the view only when the cursor gets that close to an edge.
pub fn scroll_offset(prev: usize, selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    let pad = 5.min(visible.saturating_sub(1) / 2);
    let mut offset = prev;
    if selected < offset + pad {
        offset = selected.saturating_sub(pad);
    } else if selected + pad >= offset + visible {
        offset = selected + pad + 1 - visible;
    }
    offset.min(total.saturating_sub(visible))
}

fn render_table(frame: &mut Frame, area: Rect, app: &App, lines: &[DisplayLine]) {
    let header_cells = ["MARK", "VERDICT", "YEAR", "COMPANY", "TITLE", "REASON"]
        .into_iter()
        .map(|h| Span::styled(h, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
    let header = TableRow::new(header_cells).bottom_margin(0);

    let view = app.visible_rows();
    let loc_rows = views::merge_locations(view);
    let groups = if app.tab == Tab::Jobs {
        views::group_view(loc_rows.clone())
    } else {
        Vec::new()
    };

    let rows: Vec<TableRow> = lines
        .iter()
        .map(|line| match *line {
            DisplayLine::Header(g_idx) => {
                let group = &groups[g_idx];
                let newest = group.rows[0].newest();
                let any_starred = group.rows.iter().any(|r| r.marks().starred);
                let any_applied = group.rows.iter().any(|r| r.marks().applied_at.is_some());
                let any_new = group.rows.iter().any(|r| r.is_new());
                let all_closed = group.rows.iter().all(|r| r.is_closed());

                let mark_cell = format_mark_cell(any_starred, any_applied, any_new, all_closed);

                let (verdict_str, verdict_color) = match newest.judgement.verdict {
                    Verdict::Apply => ("apply", Color::Green),
                    Verdict::Maybe => ("maybe", Color::Yellow),
                    Verdict::No => ("no", Color::Red),
                };

                let year_str = if newest.judgement.year_confirmed {
                    "yes"
                } else {
                    "year?"
                };

                let title_str = format!("{} ×{}", newest.row.job.title, group.rows.len());

                let loc_names: Vec<String> = group
                    .rows
                    .iter()
                    // full labels ("Dayton, OH +1"), so rows that share a first city stay distinguishable
                    .map(|r| r.label())
                    .collect();
                let reason_str = format!("{} locations: {}", group.rows.len(), loc_names.join("; "));

                let verdict_span = Span::styled(verdict_str, Style::default().fg(verdict_color));
                let year_span = Span::styled(year_str, Style::default().fg(Color::White));
                let company_span = Span::styled(truncate_chars(&newest.row.job.company, 20), Style::default().fg(Color::White));
                let title_span = Span::styled(truncate_chars(&title_str, 35), Style::default().fg(Color::White));
                let reason_span = Span::styled(truncate_chars(&reason_str, 60), Style::default().fg(Color::DarkGray));

                TableRow::new(vec![
                    mark_cell,
                    verdict_span.into(),
                    year_span.into(),
                    company_span.into(),
                    title_span.into(),
                    reason_span.into(),
                ])
            }
            DisplayLine::Child(g_idx, loc_idx) => {
                let loc_row = &groups[g_idx].rows[loc_idx];
                let m = loc_row.marks();
                let mark_cell = format_mark_cell(m.starred, m.applied_at.is_some(), loc_row.is_new(), loc_row.is_closed());

                let verdict_span = Span::raw("");
                let year_span = Span::raw("");
                let company_span = Span::raw("");
                let title_span = Span::styled(truncate_chars(&format!("└ {}", loc_row.label()), 35), Style::default().fg(Color::White));
                let reason_span = Span::styled(truncate_chars(&loc_row.newest().judgement.reason, 60), Style::default().fg(Color::DarkGray));

                TableRow::new(vec![
                    mark_cell,
                    verdict_span.into(),
                    year_span.into(),
                    company_span.into(),
                    title_span.into(),
                    reason_span.into(),
                ])
            }
            DisplayLine::Plain(idx) => {
                let loc_row = if app.tab == Tab::Jobs {
                    &groups[idx].rows[0]
                } else {
                    &loc_rows[idx]
                };
                let newest = loc_row.newest();

                let m = loc_row.marks();
                let mark_cell = format_mark_cell(m.starred, m.applied_at.is_some(), loc_row.is_new(), loc_row.is_closed());

                let (verdict_str, verdict_color) = match newest.judgement.verdict {
                    Verdict::Apply => ("apply", Color::Green),
                    Verdict::Maybe => ("maybe", Color::Yellow),
                    Verdict::No => ("no", Color::Red),
                };

                let year_str = if newest.judgement.year_confirmed {
                    "yes"
                } else {
                    "year?"
                };

                let title_str = if app.tab == Tab::Jobs {
                    newest.row.job.title.clone()
                } else {
                    format!("{} · {}", newest.row.job.title, loc_row.label())
                };

                let verdict_span = Span::styled(verdict_str, Style::default().fg(verdict_color));
                let year_span = Span::styled(year_str, Style::default().fg(Color::White));
                let company_span = Span::styled(truncate_chars(&newest.row.job.company, 20), Style::default().fg(Color::White));
                let title_span = Span::styled(truncate_chars(&title_str, 35), Style::default().fg(Color::White));
                let reason_span = Span::styled(truncate_chars(&newest.judgement.reason, 60), Style::default().fg(Color::DarkGray));

                TableRow::new(vec![
                    mark_cell,
                    verdict_span.into(),
                    year_span.into(),
                    company_span.into(),
                    title_span.into(),
                    reason_span.into(),
                ])
            }
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
        // explicit fg: row spans use DarkGray text, which vanished on the DarkGray highlight bar
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    // borders (2) + header row (1)
    let visible = area.height.saturating_sub(3) as usize;
    let offset = scroll_offset(app.table_offset.get(), app.selected, visible, lines.len());
    app.table_offset.set(offset);
    let mut state = TableState::default().with_offset(offset).with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App, lines: &[DisplayLine]) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let Some(&selected_line) = lines.get(app.selected) else {
        frame.render_widget(Paragraph::new("").block(block), area);
        return;
    };

    let view = app.visible_rows();
    let loc_rows = views::merge_locations(view);

    match selected_line {
        DisplayLine::Header(g_idx) => {
            let groups = views::group_view(loc_rows);
            let Some(group) = groups.get(g_idx) else {
                frame.render_widget(Paragraph::new("").block(block), area);
                return;
            };
            let newest = group.rows[0].newest();
            let (verdict_str, verdict_color) = match newest.judgement.verdict {
                Verdict::Apply => ("apply", Color::Green),
                Verdict::Maybe => ("maybe", Color::Yellow),
                Verdict::No => ("no", Color::Red),
            };

            let title_line = Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{} — {}", newest.row.job.company, newest.row.job.title),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled(
                    format!("{} locations · ", group.rows.len()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(verdict_str, Style::default().fg(verdict_color)),
            ]);

            let mut lines = vec![title_line];

            for loc_row in &group.rows {
                let m = loc_row.marks();
                let mut prefix = String::new();
                if m.starred {
                    prefix.push('★');
                }
                if m.applied_at.is_some() {
                    prefix.push('✓');
                }
                if loc_row.is_new() {
                    prefix.push('N');
                }
                let closed = loc_row.is_closed();
                let char_count = prefix.chars().count() + if closed { 1 } else { 0 };
                let pad = 4usize.saturating_sub(char_count);

                let mut spans = vec![Span::raw(" ")];
                if !prefix.is_empty() {
                    spans.push(Span::styled(
                        prefix,
                        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                    ));
                }
                if closed {
                    spans.push(Span::styled(
                        "✗",
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                    ));
                }
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(loc_row.label(), Style::default().fg(Color::White)));
                lines.push(Line::from(spans));
            }

            let paragraph = Paragraph::new(lines).block(block);
            frame.render_widget(paragraph, area);
        }
        DisplayLine::Child(g_idx, loc_idx) => {
            let groups = views::group_view(loc_rows);
            let Some(loc_row) = groups.get(g_idx).and_then(|g| g.rows.get(loc_idx)) else {
                frame.render_widget(Paragraph::new("").block(block), area);
                return;
            };
            render_loc_row_detail(frame, area, block, loc_row);
        }
        DisplayLine::Plain(idx) => {
            if app.tab == Tab::Jobs {
                let groups = views::group_view(loc_rows);
                let Some(loc_row) = groups.get(idx).and_then(|g| g.rows.get(0)) else {
                    frame.render_widget(Paragraph::new("").block(block), area);
                    return;
                };
                render_loc_row_detail(frame, area, block, loc_row);
            } else {
                let Some(loc_row) = loc_rows.get(idx) else {
                    frame.render_widget(Paragraph::new("").block(block), area);
                    return;
                };
                render_loc_row_detail(frame, area, block, loc_row);
            }
        }
    }
}

fn render_loc_row_detail(frame: &mut Frame, area: Rect, block: Block, loc_row: &LocRow) {
    let newest = loc_row.newest();
    let posted_str = format_unix_date(newest.row.date_posted);
    let provider_str = newest.row.job.provider.as_str();
    let status_str = newest.row.status.as_str();

    let title_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("{} — {}", newest.row.job.company, newest.row.job.title),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            format!("posted {posted_str} · {provider_str} · {status_str}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let format_fact_line = |name: &'static str, val: &str, ev: Option<(&str, &str)>| -> Line<'static> {
        let mut spans = vec![
            Span::raw(" "),
            Span::styled(format!("{:<13}", name), Style::default().fg(Color::Cyan)),
            Span::styled(format!("{:<14}", val), Style::default().fg(Color::White)),
        ];
        if let Some((src, snippet)) = ev {
            spans.push(Span::styled(
                format!("{}: \"{}\"", src, truncate_chars(snippet, 80)),
                Style::default().fg(Color::DarkGray),
            ));
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

    if let Some(ref f) = newest.row.facts {
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

    // locations: a; b; c
    let mut cities: Vec<String> = Vec::new();
    for m in &loc_row.members {
        for loc in &m.row.locations {
            let trimmed = loc.trim();
            if !trimmed.is_empty() && !cities.iter().any(|c| c.eq_ignore_ascii_case(trimmed)) {
                cities.push(trimmed.to_string());
            }
        }
    }
    if !cities.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled("locations: ", Style::default().fg(Color::Cyan)),
            Span::styled(cities.join("; "), Style::default().fg(Color::White)),
        ]));
    }

    // every link (one per line, truncate to width)
    for m in &loc_row.members {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate_chars(&m.row.job.url, area.width.saturating_sub(4) as usize),
                Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            ),
        ]));
    }

    // location is picked in the application form
    if loc_row.members.iter().any(|m| m.row.asks_location) {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "location is picked in the application form",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = if let Some(ref msg) = app.status_msg {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(msg, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ])
    } else if let Some(ref prog) = app.progress {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(prog, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
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
            Span::styled("A", Style::default().fg(Color::Cyan)),
            Span::raw(" open+apply  "),
            Span::styled("o", Style::default().fg(Color::Cyan)),
            Span::raw(" open  "),
            Span::styled("r", Style::default().fg(Color::Cyan)),
            Span::raw(" refresh  "),
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
        Line::from("  A                  Open newest link and mark applied"),
        Line::from("  o                  Open posting URL in browser"),
        Line::from("  r                  Refresh jobs (sync + scan)"),
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
mod scroll_tests {
    use super::scroll_offset;
    #[test]
    fn scroll_keeps_margin() {
        // 20 visible rows, 100 lines: moving down from the top scrolls once the cursor is 5 from the bottom
        assert_eq!(scroll_offset(0, 14, 20, 100), 0);
        assert_eq!(scroll_offset(0, 15, 20, 100), 1);
        assert_eq!(scroll_offset(1, 16, 20, 100), 2);
        // moving back up keeps 5 rows above
        assert_eq!(scroll_offset(30, 34, 20, 100), 29);
        assert_eq!(scroll_offset(30, 40, 20, 100), 30);
        // edges: top and bottom of the list are reachable, offset never past the end
        assert_eq!(scroll_offset(0, 0, 20, 100), 0);
        assert_eq!(scroll_offset(80, 99, 20, 100), 80);
        assert_eq!(scroll_offset(0, 3, 20, 5), 0);
        assert_eq!(scroll_offset(0, 3, 0, 5), 0);
    }
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
            llm_facts: None,
            date_posted: 1000,
            active,
            marks,
            is_new: false,
            locations: vec![],
            asks_location: false,
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

    #[test]
    fn renders_group_header_and_children() {
        let mut r1 = make_row(1, "Acme", "SWE", true, Marks::default());
        r1.locations = vec!["SF".into()];
        let mut r2 = make_row(2, "Acme", "SWE", true, Marks::default());
        r2.locations = vec!["NYC".into()];
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2], &cfg);
        let backend = TestBackend::new(150, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = buffer_to_string(terminal.backend().buffer());
        assert!(buf.contains("×2"));
        assert!(!buf.contains("└ "));

        app.on_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::empty(),
        ));
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf2 = buffer_to_string(terminal.backend().buffer());
        assert!(buf2.contains("×2"));
        assert!(buf2.contains("└ "));
    }

    #[test]
    fn renders_links_count() {
        let mut r1 = make_row(1, "Beta", "Dev", true, Marks::default());
        r1.locations = vec!["Salt Lake City, UT".into()];
        let mut r2 = make_row(2, "Beta", "Dev", true, Marks::default());
        r2.locations = vec!["Salt Lake City, UT".into()];
        let mut r3 = make_row(3, "Beta", "Dev", true, Marks::default());
        r3.locations = vec!["Denver, CO".into()];
        let cfg = test_config();
        let mut app = App::new(vec![r1, r2, r3], &cfg);
        // Expand group in Jobs tab
        app.on_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::empty(),
        ));
        let backend = TestBackend::new(150, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = buffer_to_string(terminal.backend().buffer());
        assert!(buf.contains("(2 links)"));
    }

    #[test]
    fn footer_progress_and_key_hints() {
        let r1 = make_row(1, "Beta", "Dev", true, Marks::default());
        let cfg = test_config();
        let mut app = App::new(vec![r1], &cfg);
        let backend = TestBackend::new(150, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Default: contains "r" and "refresh"
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = buffer_to_string(terminal.backend().buffer());
        assert!(buf.contains("r") && buf.contains("refresh"));

        // 2. Progress set: contains progress text
        app.progress = Some("syncing\u{2026}".to_string());
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf2 = buffer_to_string(terminal.backend().buffer());
        assert!(buf2.contains("syncing\u{2026}"));

        // 3. Status msg set: overrides progress
        app.status_msg = Some("a status message".to_string());
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf3 = buffer_to_string(terminal.backend().buffer());
        assert!(buf3.contains("a status message"));
    }

    #[test]
    fn renders_closed_badge() {
        let r1 = make_row(
            1,
            "OldCorp",
            "SWE",
            false,
            Marks {
                starred: false,
                hidden: false,
                applied_at: Some(100),
            },
        );
        let cfg = test_config();
        let mut app = App::new(vec![r1], &cfg);
        app.tab = Tab::Applied;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buffer_str = buffer_to_string(terminal.backend().buffer());
        assert!(buffer_str.contains("✗"));
    }
}
