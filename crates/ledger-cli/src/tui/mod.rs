mod app;
mod daemon;
mod event;
mod ui;

use std::io;

use anyhow::Result;
use crossterm::{
    event::{KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use app::{App, DaemonStatus, Focus, InputMode};
use daemon::DaemonMsg;
use event::TermEvent;

/// Run the TUI. Called from `ledger tui`.
pub async fn run(socket_path: String) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let result = run_app(&mut terminal, socket_path).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    socket_path: String,
) -> Result<()> {
    let mut app = App::new();

    let mut term_events = event::spawn_event_reader();
    let mut daemon_rx = daemon::spawn_daemon_bridge(socket_path);

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        tokio::select! {
            Some(term_event) = term_events.recv() => {
                match term_event {
                    TermEvent::Key(key) => handle_key(&mut app, key),
                    TermEvent::Resize => {}
                    TermEvent::Tick => {}
                }
            }
            Some(daemon_msg) = daemon_rx.recv() => {
                handle_daemon_msg(&mut app, daemon_msg);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match app.input_mode {
        InputMode::Editing => handle_key_editing(app, key),
        InputMode::Normal => handle_key_normal(app, key),
    }
}

fn handle_key_editing(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::EventList;
        }
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            app.focus = Focus::EventList;
            app.rebuild_filter();
        }
        KeyCode::Char(c) => {
            match app.focus {
                Focus::FilterSource => app.filter_source.push(c),
                Focus::FilterType => app.filter_type.push(c),
                _ => {}
            }
            app.rebuild_filter();
        }
        KeyCode::Backspace => {
            match app.focus {
                Focus::FilterSource => { app.filter_source.pop(); }
                Focus::FilterType => { app.filter_type.pop(); }
                _ => {}
            }
            app.rebuild_filter();
        }
        _ => {}
    }
}

fn handle_key_normal(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,

        KeyCode::Char('j') | KeyCode::Down => {
            if app.focus == Focus::Detail {
                app.detail_scroll_down();
            } else {
                app.select_next();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.focus == Focus::Detail {
                app.detail_scroll_up();
            } else {
                app.select_prev();
            }
        }
        KeyCode::Char('g') => app.select_first(),
        KeyCode::Char('G') => app.select_last(),

        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::EventList => Focus::Detail,
                Focus::Detail => Focus::EventList,
                Focus::FilterSource => Focus::EventList,
                Focus::FilterType => Focus::EventList,
            };
        }

        KeyCode::Enter | KeyCode::Right => {
            if app.focus == Focus::EventList {
                app.focus = Focus::Detail;
                app.detail_scroll = 0;
            }
        }

        KeyCode::Left | KeyCode::Esc => {
            if app.focus == Focus::Detail {
                app.focus = Focus::EventList;
            }
        }

        KeyCode::Char('/') => {
            app.focus = Focus::FilterSource;
            app.input_mode = InputMode::Editing;
        }
        KeyCode::Char('?') => {
            app.focus = Focus::FilterType;
            app.input_mode = InputMode::Editing;
        }

        KeyCode::Char('c') => {
            app.filter_source.clear();
            app.filter_type.clear();
            app.rebuild_filter();
        }

        KeyCode::Char('p') => {
            if app.paused {
                app.unpause();
            } else {
                app.paused = true;
            }
        }

        _ => {}
    }
}

fn handle_daemon_msg(app: &mut App, msg: DaemonMsg) {
    match msg {
        DaemonMsg::History(events) => {
            app.load_events(events);
            app.daemon_status = DaemonStatus::Connected {
                uptime_seconds: 0,
                event_count: app.total_count() as u64,
                version: String::new(),
            };
        }
        DaemonMsg::LiveEvent(event) => {
            app.push_event(event);
        }
        DaemonMsg::Health(health) => {
            app.daemon_status = DaemonStatus::Connected {
                uptime_seconds: health.uptime_seconds,
                event_count: health.event_count,
                version: health.version,
            };
        }
        DaemonMsg::Error(err) => {
            app.daemon_status = DaemonStatus::Disconnected(err);
        }
    }
}
