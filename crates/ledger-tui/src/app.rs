use std::collections::BTreeSet;

use ledger_client::Event;

/// Which pane is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    EventList,
    Detail,
    FilterSource,
    FilterType,
}

/// Application input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

/// Daemon connection status.
#[derive(Debug, Clone)]
pub enum DaemonStatus {
    Connecting,
    Connected {
        uptime_seconds: u64,
        event_count: u64,
        version: String,
    },
    Disconnected(String),
}

/// Core application state — the single source of truth for the TUI.
pub struct App {
    /// All events we know about, newest first.
    pub events: Vec<Event>,
    /// Index of the currently selected event in the (filtered) list.
    pub selected: usize,
    /// Vertical scroll offset for the detail pane.
    pub detail_scroll: u16,
    /// Currently active pane.
    pub focus: Focus,
    /// Input mode (normal navigation vs text editing for filters).
    pub input_mode: InputMode,
    /// Source filter text.
    pub filter_source: String,
    /// Event-type filter text.
    pub filter_type: String,
    /// Cached filtered indices into `self.events`.
    pub filtered_indices: Vec<usize>,
    /// Known sources (for display hints).
    pub known_sources: BTreeSet<String>,
    /// Known event types (for display hints).
    pub known_types: BTreeSet<String>,
    /// Daemon connection status.
    pub daemon_status: DaemonStatus,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Whether live streaming is paused (freeze the view).
    pub paused: bool,
    /// Events received while paused (buffered for when we unpause).
    pub paused_buffer: Vec<Event>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            events: Vec::new(),
            selected: 0,
            detail_scroll: 0,
            focus: Focus::EventList,
            input_mode: InputMode::Normal,
            filter_source: String::new(),
            filter_type: String::new(),
            filtered_indices: Vec::new(),
            known_sources: BTreeSet::new(),
            known_types: BTreeSet::new(),
            daemon_status: DaemonStatus::Connecting,
            should_quit: false,
            paused: false,
            paused_buffer: Vec::new(),
        };
        app.rebuild_filter();
        app
    }

    /// Add a new event (from the live stream or initial query).
    /// Inserts at the front (newest first) and updates filter cache.
    pub fn push_event(&mut self, event: Event) {
        self.known_sources.insert(event.source.clone());
        self.known_types.insert(event.event_type.clone());

        if self.paused {
            self.paused_buffer.push(event);
            return;
        }

        self.events.insert(0, event);
        self.rebuild_filter();
    }

    /// Bulk-load events (from initial Query). Assumes events are already sorted newest-first.
    pub fn load_events(&mut self, events: Vec<Event>) {
        for e in &events {
            self.known_sources.insert(e.source.clone());
            self.known_types.insert(e.event_type.clone());
        }
        self.events = events;
        self.rebuild_filter();
        self.selected = 0;
        self.detail_scroll = 0;
    }

    /// Flush paused buffer when unpausing.
    pub fn unpause(&mut self) {
        self.paused = false;
        let buffered: Vec<Event> = self.paused_buffer.drain(..).rev().collect();
        for event in buffered {
            self.events.insert(0, event);
        }
        self.rebuild_filter();
    }

    /// Rebuild the filtered index list based on current filter strings.
    pub fn rebuild_filter(&mut self) {
        self.filtered_indices = self
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                let source_ok = self.filter_source.is_empty()
                    || e.source
                        .to_lowercase()
                        .contains(&self.filter_source.to_lowercase());
                let type_ok = self.filter_type.is_empty()
                    || e.event_type
                        .to_lowercase()
                        .contains(&self.filter_type.to_lowercase());
                source_ok && type_ok
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len() - 1;
        }
    }

    /// Get the currently selected event (if any).
    pub fn selected_event(&self) -> Option<&Event> {
        self.filtered_indices
            .get(self.selected)
            .and_then(|&idx| self.events.get(idx))
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.detail_scroll = 0;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() && self.selected < self.filtered_indices.len() - 1 {
            self.selected += 1;
            self.detail_scroll = 0;
        }
    }

    /// Jump to top of list.
    pub fn select_first(&mut self) {
        self.selected = 0;
        self.detail_scroll = 0;
    }

    /// Jump to bottom of list.
    pub fn select_last(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = self.filtered_indices.len() - 1;
            self.detail_scroll = 0;
        }
    }

    /// Scroll detail pane up.
    pub fn detail_scroll_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
    }

    /// Scroll detail pane down.
    pub fn detail_scroll_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
    }

    /// Count of visible (filtered) events.
    pub fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Total event count.
    pub fn total_count(&self) -> usize {
        self.events.len()
    }
}
