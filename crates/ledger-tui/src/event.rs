use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyEvent, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;

/// Terminal events distilled to what the app cares about.
#[derive(Debug)]
pub enum TermEvent {
    Key(KeyEvent),
    Resize,
    Tick,
}

/// Reads crossterm events in an async task, forwards them through a channel.
///
/// Also sends periodic Tick events for UI refresh (e.g. updating elapsed time displays).
pub fn spawn_event_reader() -> mpsc::UnboundedReceiver<TermEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                maybe_event = reader.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) => {
                            // crossterm fires Press + Release on some platforms;
                            // we only care about Press.
                            if key.kind == KeyEventKind::Press {
                                if tx.send(TermEvent::Key(key)).is_err() {
                                    break;
                                }
                            }
                        }
                        Some(Ok(Event::Resize(_, _))) => {
                            if tx.send(TermEvent::Resize).is_err() {
                                break;
                            }
                        }
                        Some(Ok(_)) => {} // mouse, focus, paste — ignore
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                _ = tick_interval.tick() => {
                    if tx.send(TermEvent::Tick).is_err() {
                        break;
                    }
                }
            }
        }
    });

    rx
}
