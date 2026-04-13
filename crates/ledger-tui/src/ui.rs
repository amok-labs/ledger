use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation,
              ScrollbarState, Table, TableState, Wrap},
    Frame,
};

use crate::app::{App, DaemonStatus, Focus, InputMode};

/// Render the entire UI.
pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // filter bar
            Constraint::Min(10),  // main content
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    draw_filter_bar(f, app, chunks[0]);
    draw_main_content(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
}

fn draw_filter_bar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Source filter
    let source_style = match (app.focus, app.input_mode) {
        (Focus::FilterSource, InputMode::Editing) => {
            Style::default().fg(Color::Yellow)
        }
        (Focus::FilterSource, _) => Style::default().fg(Color::Cyan),
        _ => Style::default().fg(Color::DarkGray),
    };
    let source_text = if app.filter_source.is_empty() {
        "all".to_string()
    } else {
        app.filter_source.clone()
    };
    let source_block = Block::default()
        .title(" Source (/) ")
        .borders(Borders::ALL)
        .border_style(source_style);
    let source_para = Paragraph::new(source_text).block(source_block);
    f.render_widget(source_para, chunks[0]);

    // Type filter
    let type_style = match (app.focus, app.input_mode) {
        (Focus::FilterType, InputMode::Editing) => {
            Style::default().fg(Color::Yellow)
        }
        (Focus::FilterType, _) => Style::default().fg(Color::Cyan),
        _ => Style::default().fg(Color::DarkGray),
    };
    let type_text = if app.filter_type.is_empty() {
        "all".to_string()
    } else {
        app.filter_type.clone()
    };
    let type_block = Block::default()
        .title(" Type (?) ")
        .borders(Borders::ALL)
        .border_style(type_style);
    let type_para = Paragraph::new(type_text).block(type_block);
    f.render_widget(type_para, chunks[1]);

    // Show cursor when editing
    if app.input_mode == InputMode::Editing {
        match app.focus {
            Focus::FilterSource => {
                f.set_cursor_position((
                    chunks[0].x + app.filter_source.len() as u16 + 1,
                    chunks[0].y + 1,
                ));
            }
            Focus::FilterType => {
                f.set_cursor_position((
                    chunks[1].x + app.filter_type.len() as u16 + 1,
                    chunks[1].y + 1,
                ));
            }
            _ => {}
        }
    }
}

fn draw_main_content(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_event_table(f, app, chunks[0]);
    draw_detail_pane(f, app, chunks[1]);
}

fn draw_event_table(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Focus::EventList {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let paused_indicator = if app.paused { " [PAUSED] " } else { "" };
    let title = format!(
        " Events ({}/{}) {}",
        app.filtered_count(),
        app.total_count(),
        paused_indicator
    );

    let header = Row::new(vec![
        Cell::from("Time").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Source").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Type").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::White))
    .height(1);

    let rows: Vec<Row> = app
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(display_idx, &event_idx)| {
            let event = &app.events[event_idx];
            let time = format_timestamp(event);
            let style = if display_idx == app.selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                source_color(&event.source)
            };

            Row::new(vec![
                Cell::from(time),
                Cell::from(event.source.clone()),
                Cell::from(event.event_type.clone()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = TableState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(table, area, &mut state);

    // Scrollbar
    if !app.filtered_indices.is_empty() {
        let mut scrollbar_state = ScrollbarState::new(app.filtered_indices.len())
            .position(app.selected);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

fn draw_detail_pane(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Focus::Detail {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Detail ")
        .borders(Borders::ALL)
        .border_style(border_style);

    match app.selected_event() {
        Some(event) => {
            let time = format_timestamp_full(event);
            let mut lines: Vec<Line> = vec![
                Line::from(vec![
                    Span::styled("ID:     ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&event.id),
                ]),
                Line::from(vec![
                    Span::styled("Time:   ", Style::default().fg(Color::DarkGray)),
                    Span::raw(time),
                ]),
                Line::from(vec![
                    Span::styled("Source: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&event.source, source_color(&event.source)),
                ]),
                Line::from(vec![
                    Span::styled("Type:   ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&event.event_type),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "── Payload ──",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            // Pretty-print the JSON payload
            let payload_lines = format_payload(&event.payload);
            for line in payload_lines {
                lines.push(Line::from(line));
            }

            let paragraph = Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((app.detail_scroll, 0));
            f.render_widget(paragraph, area);
        }
        None => {
            let paragraph = Paragraph::new("No event selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(paragraph, area);
        }
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let (status_text, status_color) = match &app.daemon_status {
        DaemonStatus::Connecting => ("connecting...".to_string(), Color::Yellow),
        DaemonStatus::Connected {
            uptime_seconds,
            event_count,
            version,
        } => {
            let uptime = format_duration(*uptime_seconds);
            (
                format!(
                    "● connected  │  v{}  │  {} events  │  uptime {}  │  q:quit  /:source  ?:type  p:pause  Tab:focus",
                    version, event_count, uptime
                ),
                Color::Green,
            )
        }
        DaemonStatus::Disconnected(msg) => {
            (format!("✗ disconnected: {}", msg), Color::Red)
        }
    };

    let bar = Paragraph::new(Line::from(vec![Span::styled(
        status_text,
        Style::default().fg(status_color),
    )]))
    .style(Style::default().bg(Color::Rgb(30, 30, 30)));

    f.render_widget(bar, area);
}

// ── Helpers ────────────────────────────────────────────────

fn format_timestamp(event: &ledger_client::Event) -> String {
    event
        .timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_default();
            dt.format("%H:%M:%S").to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

fn format_timestamp_full(event: &ledger_client::Event) -> String {
    event
        .timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_default();
            dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

fn format_payload(payload: &str) -> Vec<String> {
    // Try to pretty-print as JSON
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(val) => serde_json::to_string_pretty(&val)
            .unwrap_or_else(|_| payload.to_string())
            .lines()
            .map(String::from)
            .collect(),
        Err(_) => payload.lines().map(String::from).collect(),
    }
}

fn format_duration(seconds: u64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{}h {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

fn source_color(source: &str) -> Style {
    match source {
        "pds" => Style::default().fg(Color::Blue),
        "haro" => Style::default().fg(Color::Magenta),
        _ => Style::default().fg(Color::White),
    }
}
