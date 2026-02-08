use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

use crate::config::NostaroConfig;

pub struct CacheDb {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct CachedEvent {
    pub id: String,
    pub pubkey: String,
    pub kind: u16,
    pub content: String,
    pub created_at: i64,
    pub tags_json: String,
    pub raw_json: String,
}

#[derive(Debug, Clone)]
pub struct CachedProfile {
    pub pubkey: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub updated_at: i64,
}

impl CacheDb {
    pub fn open() -> Result<Self> {
        let db_path = Self::db_path();
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&db_path)?;
        let db = Self { conn };
        db.init_tables()?;
        Ok(db)
    }

    fn db_path() -> PathBuf {
        NostaroConfig::config_dir().join("cache.db")
    }

    fn init_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                pubkey TEXT NOT NULL,
                kind INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                tags_json TEXT NOT NULL,
                raw_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS profiles (
                pubkey TEXT PRIMARY KEY,
                name TEXT,
                display_name TEXT,
                about TEXT,
                picture TEXT,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_kind ON events(kind);
            CREATE INDEX IF NOT EXISTS idx_events_pubkey ON events(pubkey);
            CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);",
        )?;
        Ok(())
    }

    pub fn store_event(
        &self,
        id: &str,
        pubkey: &str,
        kind: u16,
        content: &str,
        created_at: i64,
        tags_json: &str,
        raw_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO events (id, pubkey, kind, content, created_at, tags_json, raw_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, pubkey, kind as i64, content, created_at, tags_json, raw_json],
        )?;
        Ok(())
    }

    pub fn get_event(&self, id: &str) -> Result<Option<CachedEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, pubkey, kind, content, created_at, tags_json, raw_json FROM events WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(CachedEvent {
                id: row.get(0)?,
                pubkey: row.get(1)?,
                kind: row.get::<_, i64>(2)? as u16,
                content: row.get(3)?,
                created_at: row.get(4)?,
                tags_json: row.get(5)?,
                raw_json: row.get(6)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn store_profile(
        &self,
        pubkey: &str,
        name: Option<&str>,
        display_name: Option<&str>,
        about: Option<&str>,
        picture: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO profiles (pubkey, name, display_name, about, picture, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![pubkey, name, display_name, about, picture, now],
        )?;
        Ok(())
    }

    pub fn get_profile(&self, pubkey: &str) -> Result<Option<CachedProfile>> {
        let mut stmt = self.conn.prepare(
            "SELECT pubkey, name, display_name, about, picture, updated_at FROM profiles WHERE pubkey = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![pubkey])?;
        if let Some(row) = rows.next()? {
            Ok(Some(CachedProfile {
                pubkey: row.get(0)?,
                name: row.get(1)?,
                display_name: row.get(2)?,
                about: row.get(3)?,
                picture: row.get(4)?,
                updated_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM events; DELETE FROM profiles;")?;
        Ok(())
    }

    pub fn stats(&self) -> Result<(usize, usize)> {
        let events: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        let profiles: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM profiles", [], |row| row.get(0))?;
        Ok((events as usize, profiles as usize))
    }

    pub fn recent_events(&self, kind: u16, limit: usize) -> Result<Vec<CachedEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, pubkey, kind, content, created_at, tags_json, raw_json FROM events WHERE kind = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![kind as i64, limit as i64], |row| {
            Ok(CachedEvent {
                id: row.get(0)?,
                pubkey: row.get(1)?,
                kind: row.get::<_, i64>(2)? as u16,
                content: row.get(3)?,
                created_at: row.get(4)?,
                tags_json: row.get(5)?,
                raw_json: row.get(6)?,
            })
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> CacheDb {
        let conn = Connection::open_in_memory().unwrap();
        let db = CacheDb { conn };
        db.init_tables().unwrap();
        db
    }

    #[test]
    fn test_store_and_get_event() {
        let db = test_db();
        db.store_event("abc123", "pubkey1", 1, "hello", 1000, "[]", "{}")
            .unwrap();
        let event = db.get_event("abc123").unwrap().unwrap();
        assert_eq!(event.id, "abc123");
        assert_eq!(event.content, "hello");
        assert_eq!(event.kind, 1);
    }

    #[test]
    fn test_get_nonexistent_event() {
        let db = test_db();
        let event = db.get_event("nonexistent").unwrap();
        assert!(event.is_none());
    }

    #[test]
    fn test_store_and_get_profile() {
        let db = test_db();
        db.store_profile("pk1", Some("alice"), Some("Alice"), Some("bio"), None)
            .unwrap();
        let profile = db.get_profile("pk1").unwrap().unwrap();
        assert_eq!(profile.name.unwrap(), "alice");
        assert_eq!(profile.display_name.unwrap(), "Alice");
        assert!(profile.picture.is_none());
    }

    #[test]
    fn test_recent_events() {
        let db = test_db();
        db.store_event("e1", "pk", 1, "first", 100, "[]", "{}")
            .unwrap();
        db.store_event("e2", "pk", 1, "second", 200, "[]", "{}")
            .unwrap();
        db.store_event("e3", "pk", 1, "third", 300, "[]", "{}")
            .unwrap();
        let events = db.recent_events(1, 2).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].content, "third");
        assert_eq!(events[1].content, "second");
    }
}
