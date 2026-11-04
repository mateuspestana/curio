//! SQLite persistence for Curió.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::{Change, Snapshot, Watch, WatchStatus, MAX_SNAPSHOTS_PER_WATCH};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open_path(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("criar diretório {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("abrir banco {}", path.display()))?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS watches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                interval_secs INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                last_checked_at TEXT,
                last_status TEXT NOT NULL DEFAULT 'waiting',
                last_error TEXT,
                unread_change INTEGER NOT NULL DEFAULT 0,
                consecutive_failures INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                watch_id INTEGER NOT NULL REFERENCES watches(id) ON DELETE CASCADE,
                fetched_at TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                normalized_text TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                watch_id INTEGER NOT NULL REFERENCES watches(id) ON DELETE CASCADE,
                detected_at TEXT NOT NULL,
                diff_text TEXT NOT NULL,
                seen INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_watch ON snapshots(watch_id, fetched_at DESC);
            CREATE INDEX IF NOT EXISTS idx_changes_watch ON changes(watch_id, detected_at DESC);
            "#,
        )?;
        Ok(())
    }

    pub fn list_watches(&self) -> Result<Vec<Watch>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, label, interval_secs, created_at, last_checked_at,
                    last_status, last_error, unread_change, consecutive_failures
             FROM watches ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Watch {
                id: row.get(0)?,
                url: row.get(1)?,
                label: row.get(2)?,
                interval_secs: row.get::<_, i64>(3)? as u64,
                created_at: parse_dt(&row.get::<_, String>(4)?)?,
                last_checked_at: row
                    .get::<_, Option<String>>(5)?
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                last_status: WatchStatus::parse(&row.get::<_, String>(6)?),
                last_error: row.get(7)?,
                unread_change: row.get::<_, i64>(8)? != 0,
                consecutive_failures: row.get::<_, i64>(9)? as u32,
                next_check_at: None,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_watch(&self, id: i64) -> Result<Option<Watch>> {
        self.conn
            .query_row(
                "SELECT id, url, label, interval_secs, created_at, last_checked_at,
                        last_status, last_error, unread_change, consecutive_failures
                 FROM watches WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Watch {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        label: row.get(2)?,
                        interval_secs: row.get::<_, i64>(3)? as u64,
                        created_at: parse_dt(&row.get::<_, String>(4)?)?,
                        last_checked_at: row
                            .get::<_, Option<String>>(5)?
                            .as_deref()
                            .map(parse_dt)
                            .transpose()?,
                        last_status: WatchStatus::parse(&row.get::<_, String>(6)?),
                        last_error: row.get(7)?,
                        unread_change: row.get::<_, i64>(8)? != 0,
                        consecutive_failures: row.get::<_, i64>(9)? as u32,
                        next_check_at: None,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_watch(&self, url: &str, label: &str, interval_secs: u64) -> Result<Watch> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO watches (url, label, interval_secs, created_at, last_status)
             VALUES (?1, ?2, ?3, ?4, 'waiting')",
            params![url, label, interval_secs as i64, now.to_rfc3339()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_watch(id)?
            .context("watch inserido não encontrado")
    }

    pub fn update_watch(
        &self,
        id: i64,
        url: &str,
        label: &str,
        interval_secs: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE watches SET url = ?1, label = ?2, interval_secs = ?3 WHERE id = ?4",
            params![url, label, interval_secs as i64, id],
        )?;
        Ok(())
    }

    pub fn delete_watch(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM changes WHERE watch_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM snapshots WHERE watch_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM watches WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn update_watch_check(
        &self,
        id: i64,
        status: WatchStatus,
        error: Option<&str>,
        consecutive_failures: u32,
        unread_change: Option<bool>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        if let Some(unread) = unread_change {
            self.conn.execute(
                "UPDATE watches SET last_checked_at = ?1, last_status = ?2, last_error = ?3,
                 consecutive_failures = ?4, unread_change = ?5 WHERE id = ?6",
                params![
                    now,
                    status.as_str(),
                    error,
                    consecutive_failures as i64,
                    if unread { 1 } else { 0 },
                    id
                ],
            )?;
        } else {
            self.conn.execute(
                "UPDATE watches SET last_checked_at = ?1, last_status = ?2, last_error = ?3,
                 consecutive_failures = ?4 WHERE id = ?5",
                params![
                    now,
                    status.as_str(),
                    error,
                    consecutive_failures as i64,
                    id
                ],
            )?;
        }
        Ok(())
    }

    pub fn latest_snapshot(&self, watch_id: i64) -> Result<Option<Snapshot>> {
        self.conn
            .query_row(
                "SELECT id, watch_id, fetched_at, content_hash, normalized_text
                 FROM snapshots WHERE watch_id = ?1 ORDER BY fetched_at DESC, id DESC LIMIT 1",
                params![watch_id],
                |row| {
                    Ok(Snapshot {
                        id: row.get(0)?,
                        watch_id: row.get(1)?,
                        fetched_at: parse_dt(&row.get::<_, String>(2)?)?,
                        content_hash: row.get(3)?,
                        normalized_text: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_snapshot(
        &self,
        watch_id: i64,
        content_hash: &str,
        normalized_text: &str,
    ) -> Result<Snapshot> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO snapshots (watch_id, fetched_at, content_hash, normalized_text)
             VALUES (?1, ?2, ?3, ?4)",
            params![watch_id, now.to_rfc3339(), content_hash, normalized_text],
        )?;
        let id = self.conn.last_insert_rowid();
        self.trim_snapshots(watch_id)?;
        Ok(Snapshot {
            id,
            watch_id,
            fetched_at: now,
            content_hash: content_hash.to_string(),
            normalized_text: normalized_text.to_string(),
        })
    }

    fn trim_snapshots(&self, watch_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM snapshots WHERE watch_id = ?1 AND id NOT IN (
                SELECT id FROM snapshots WHERE watch_id = ?1
                ORDER BY fetched_at DESC, id DESC LIMIT ?2
             )",
            params![watch_id, MAX_SNAPSHOTS_PER_WATCH as i64],
        )?;
        Ok(())
    }

    pub fn insert_change(&self, watch_id: i64, diff_text: &str) -> Result<Change> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO changes (watch_id, detected_at, diff_text, seen) VALUES (?1, ?2, ?3, 0)",
            params![watch_id, now.to_rfc3339(), diff_text],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(Change {
            id,
            watch_id,
            detected_at: now,
            diff_text: diff_text.to_string(),
            seen: false,
        })
    }

    pub fn list_changes(&self, watch_id: i64, limit: usize) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, watch_id, detected_at, diff_text, seen
             FROM changes WHERE watch_id = ?1
             ORDER BY detected_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![watch_id, limit as i64], |row| {
            Ok(Change {
                id: row.get(0)?,
                watch_id: row.get(1)?,
                detected_at: parse_dt(&row.get::<_, String>(2)?)?,
                diff_text: row.get(3)?,
                seen: row.get::<_, i64>(4)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_recent_changes(&self, limit: usize) -> Result<Vec<(Change, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.watch_id, c.detected_at, c.diff_text, c.seen, w.label, w.url
             FROM changes c JOIN watches w ON w.id = c.watch_id
             ORDER BY c.detected_at DESC, c.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((
                Change {
                    id: row.get(0)?,
                    watch_id: row.get(1)?,
                    detected_at: parse_dt(&row.get::<_, String>(2)?)?,
                    diff_text: row.get(3)?,
                    seen: row.get::<_, i64>(4)? != 0,
                },
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn mark_changes_seen_for_watch(&self, watch_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE changes SET seen = 1 WHERE watch_id = ?1",
            params![watch_id],
        )?;
        self.conn.execute(
            "UPDATE watches SET unread_change = 0 WHERE id = ?1",
            params![watch_id],
        )?;
        Ok(())
    }

    pub fn mark_all_changes_seen(&self) -> Result<()> {
        self.conn.execute("UPDATE changes SET seen = 1", [])?;
        self.conn
            .execute("UPDATE watches SET unread_change = 0", [])?;
        Ok(())
    }
}

pub fn default_db_path() -> Result<PathBuf> {
    let home = dirs_home().context("HOME não definido")?;
    Ok(home.join(".curio").join("curio.db"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_dt(s: &str) -> std::result::Result<DateTime<Utc>, rusqlite::Error> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_and_snapshot_trim() {
        let db = Database::open_in_memory().unwrap();
        let w = db
            .insert_watch("https://example.com", "ex", 60)
            .unwrap();
        assert_eq!(w.url, "https://example.com");

        for i in 0..25 {
            db.insert_snapshot(w.id, &format!("h{i}"), &format!("text {i}"))
                .unwrap();
        }
        let latest = db.latest_snapshot(w.id).unwrap().unwrap();
        assert!(latest.normalized_text.contains("24") || latest.content_hash == "h24");

        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM snapshots WHERE watch_id = ?1",
                params![w.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, MAX_SNAPSHOTS_PER_WATCH as i64);

        db.insert_change(w.id, "- a\n+ b\n").unwrap();
        let changes = db.list_changes(w.id, 10).unwrap();
        assert_eq!(changes.len(), 1);
        db.mark_changes_seen_for_watch(w.id).unwrap();
        let w2 = db.get_watch(w.id).unwrap().unwrap();
        assert!(!w2.unread_change);

        db.delete_watch(w.id).unwrap();
        assert!(db.list_watches().unwrap().is_empty());
    }
}
