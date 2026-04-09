use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::Utc;
use prost_types::Timestamp;
use rusqlite::Connection;
use uuid::Uuid;

use crate::migrations;
use crate::proto;

/// Thread-safe SQLite event store.
pub struct EventStore {
    conn: Mutex<Connection>,
}

impl EventStore {
    /// Open or create a SQLite database at the given path with WAL mode.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = Connection::open(db_path.as_ref())
            .with_context(|| format!("Failed to open database at {:?}", db_path.as_ref()))?;

        // Enable WAL mode for concurrent reads
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;

        migrations::run_migrations(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert a new event and return the full Event with generated id and timestamp.
    pub fn insert_event(
        &self,
        source: &str,
        event_type: &str,
        payload: &str,
    ) -> Result<proto::Event> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let timestamp_str = now.to_rfc3339();

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (id, timestamp, source, event_type, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, timestamp_str, source, event_type, payload],
        )?;

        Ok(proto::Event {
            id,
            timestamp: Some(Timestamp {
                seconds: now.timestamp(),
                nanos: now.timestamp_subsec_nanos() as i32,
            }),
            source: source.to_string(),
            event_type: event_type.to_string(),
            payload: payload.to_string(),
        })
    }

    /// Query events with optional filters.
    pub fn query_events(
        &self,
        source: &str,
        event_type: &str,
        limit: i64,
        since: Option<&Timestamp>,
        until: Option<&Timestamp>,
    ) -> Result<Vec<proto::Event>> {
        let conn = self.conn.lock().unwrap();

        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if !source.is_empty() {
            conditions.push("source = ?".to_string());
            params.push(Box::new(source.to_string()));
        }
        if !event_type.is_empty() {
            conditions.push("event_type = ?".to_string());
            params.push(Box::new(event_type.to_string()));
        }
        if let Some(ts) = since {
            let dt = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_default();
            conditions.push("timestamp >= ?".to_string());
            params.push(Box::new(dt.to_rfc3339()));
        }
        if let Some(ts) = until {
            let dt = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_default();
            conditions.push("timestamp <= ?".to_string());
            params.push(Box::new(dt.to_rfc3339()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let limit_clause = if limit > 0 {
            format!(" LIMIT {}", limit)
        } else {
            String::new()
        };

        let sql = format!(
            "SELECT id, timestamp, source, event_type, payload FROM events{} ORDER BY timestamp DESC{}",
            where_clause, limit_clause
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let events = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id: String = row.get(0)?;
                let timestamp_str: String = row.get(1)?;
                let source: String = row.get(2)?;
                let event_type: String = row.get(3)?;
                let payload: String = row.get(4)?;

                let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .map(|dt| Timestamp {
                        seconds: dt.timestamp(),
                        nanos: dt.timestamp_subsec_nanos() as i32,
                    })
                    .ok();

                Ok(proto::Event {
                    id,
                    timestamp,
                    source,
                    event_type,
                    payload,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(events)
    }

    /// Get the total number of events.
    pub fn event_count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_query() {
        let store = EventStore::in_memory().unwrap();

        let event = store
            .insert_event("test", "ping", r#"{"msg":"hello"}"#)
            .unwrap();
        assert!(!event.id.is_empty());
        assert_eq!(event.source, "test");
        assert_eq!(event.event_type, "ping");

        let events = store.query_events("test", "ping", 0, None, None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);
    }

    #[test]
    fn test_query_filters() {
        let store = EventStore::in_memory().unwrap();

        store.insert_event("pds", "skill_invoked", "{}").unwrap();
        store.insert_event("pds", "agent_spawned", "{}").unwrap();
        store.insert_event("haro", "skill_invoked", "{}").unwrap();

        // Filter by source
        let events = store.query_events("pds", "", 0, None, None).unwrap();
        assert_eq!(events.len(), 2);

        // Filter by event_type
        let events = store
            .query_events("", "skill_invoked", 0, None, None)
            .unwrap();
        assert_eq!(events.len(), 2);

        // Filter by both
        let events = store
            .query_events("pds", "skill_invoked", 0, None, None)
            .unwrap();
        assert_eq!(events.len(), 1);

        // Limit
        let events = store.query_events("", "", 1, None, None).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_event_count() {
        let store = EventStore::in_memory().unwrap();
        assert_eq!(store.event_count().unwrap(), 0);

        store.insert_event("test", "ping", "{}").unwrap();
        store.insert_event("test", "pong", "{}").unwrap();
        assert_eq!(store.event_count().unwrap(), 2);
    }
}
