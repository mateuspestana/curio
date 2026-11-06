//! SQLite persistence for Curió.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::{
    AppSettings, Change, Density, Mode, Retention, Snapshot, ThemeMode, Watch, WatchStatus,
};
use crate::spec::WatchSpec;
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
        let db = Self::connect(path)?;
        db.migrate()?;
        Ok(db)
    }

    /// Abre a conexão sem migrar: WAL + busy_timeout permitem UI e monitor
    /// compartilharem o arquivo sem "database is locked".
    pub fn connect(path: &Path) -> Result<Self> {
        let conn =
            Connection::open(path).with_context(|| format!("abrir banco {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
        )?;
        Ok(Self { conn })
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

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_watch ON snapshots(watch_id, fetched_at DESC);
            CREATE INDEX IF NOT EXISTS idx_changes_watch ON changes(watch_id, detected_at DESC);
            "#,
        )?;
        self.ensure_column("watches", "selector", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("watches", "ignore_regex", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("watches", "paused", "INTEGER NOT NULL DEFAULT 0")?;
        // v0.4.0 — só ADD COLUMN, para o binário 0.3.x continuar lendo o banco.
        self.ensure_column("watches", "mode", "TEXT NOT NULL DEFAULT 'html'")?;
        self.ensure_column("watches", "condition", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("watches", "headers", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("watches", "tags", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("watches", "down_since", "TEXT")?;
        Ok(())
    }

    fn ensure_column(&self, table: &str, column: &str, decl: &str) -> Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        if !names.iter().any(|n| n == column) {
            self.conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
                [],
            )?;
        }
        Ok(())
    }

    fn map_watch(row: &rusqlite::Row<'_>) -> rusqlite::Result<Watch> {
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
            selector: row.get(10)?,
            ignore_regex: row.get(11)?,
            paused: row.get::<_, i64>(12)? != 0,
            mode: Mode::parse(&row.get::<_, String>(13)?),
            condition: row.get(14)?,
            headers: row.get(15)?,
            tags: row.get(16)?,
            down_since: row
                .get::<_, Option<String>>(17)?
                .as_deref()
                .map(parse_dt)
                .transpose()?,
            next_check_at: None,
        })
    }

    const WATCH_SELECT: &'static str =
        "SELECT id, url, label, interval_secs, created_at, last_checked_at,
            last_status, last_error, unread_change, consecutive_failures,
            selector, ignore_regex, paused,
            mode, condition, headers, tags, down_since
     FROM watches";

    pub fn list_watches(&self) -> Result<Vec<Watch>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{} ORDER BY id ASC", Self::WATCH_SELECT))?;
        let rows = stmt.query_map([], Self::map_watch)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_watch(&self, id: i64) -> Result<Option<Watch>> {
        self.conn
            .query_row(
                &format!("{} WHERE id = ?1", Self::WATCH_SELECT),
                params![id],
                Self::map_watch,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_watch(&self, spec: &WatchSpec) -> Result<Watch> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO watches (url, label, interval_secs, created_at, last_status,
                selector, ignore_regex, paused, mode, condition, headers, tags)
             VALUES (?1, ?2, ?3, ?4, 'waiting', ?5, ?6, 0, ?7, ?8, ?9, ?10)",
            params![
                spec.url,
                spec.label,
                spec.interval_secs as i64,
                now.to_rfc3339(),
                spec.selector,
                spec.ignore_regex,
                spec.mode.as_str(),
                spec.condition,
                spec.headers,
                spec.tags,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_watch(id)?.context("watch inserido não encontrado")
    }

    pub fn update_watch(&self, id: i64, spec: &WatchSpec) -> Result<()> {
        self.conn.execute(
            "UPDATE watches SET url = ?1, label = ?2, interval_secs = ?3, selector = ?4,
             ignore_regex = ?5, mode = ?6, condition = ?7, headers = ?8, tags = ?9
             WHERE id = ?10",
            params![
                spec.url,
                spec.label,
                spec.interval_secs as i64,
                spec.selector,
                spec.ignore_regex,
                spec.mode.as_str(),
                spec.condition,
                spec.headers,
                spec.tags,
                id
            ],
        )?;
        Ok(())
    }

    /// Marca (ou limpa) o início do downtime já notificado.
    pub fn set_down_since(&self, id: i64, since: Option<DateTime<Utc>>) -> Result<()> {
        self.conn.execute(
            "UPDATE watches SET down_since = ?1 WHERE id = ?2",
            params![since.map(|t| t.to_rfc3339()), id],
        )?;
        Ok(())
    }

    pub fn set_paused(&self, id: i64, paused: bool) -> Result<()> {
        let status = if paused {
            WatchStatus::Paused.as_str()
        } else {
            WatchStatus::Waiting.as_str()
        };
        self.conn.execute(
            "UPDATE watches SET paused = ?1, last_status = ?2 WHERE id = ?3",
            params![if paused { 1 } else { 0 }, status, id],
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
                params![now, status.as_str(), error, consecutive_failures as i64, id],
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
        let keep = self.load_retention()?.max_snapshots.max(1);
        self.conn.execute(
            "DELETE FROM snapshots WHERE watch_id = ?1 AND id NOT IN (
                SELECT id FROM snapshots WHERE watch_id = ?1
                ORDER BY fetched_at DESC, id DESC LIMIT ?2
             )",
            params![watch_id, keep as i64],
        )?;
        Ok(())
    }

    /// `seen = true` registra a mudança no histórico sem acender o alerta.
    pub fn insert_change(&self, watch_id: i64, diff_text: &str, seen: bool) -> Result<Change> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO changes (watch_id, detected_at, diff_text, seen) VALUES (?1, ?2, ?3, ?4)",
            params![watch_id, now.to_rfc3339(), diff_text, seen as i64],
        )?;
        let id = self.conn.last_insert_rowid();
        let max = self.load_retention()?.max_changes;
        if max > 0 {
            self.trim_changes(Some(watch_id), max)?;
        }
        Ok(Change {
            id,
            watch_id,
            detected_at: now,
            diff_text: diff_text.to_string(),
            seen,
        })
    }

    /// Mantém só as `max` mudanças mais recentes (por monitor, ou de todos).
    fn trim_changes(&self, watch_id: Option<i64>, max: u32) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM changes WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (
                        PARTITION BY watch_id ORDER BY detected_at DESC, id DESC
                    ) AS rn, watch_id FROM changes
                ) WHERE rn > ?1 AND (?2 IS NULL OR watch_id = ?2)
             )",
            params![max as i64, watch_id],
        )?;
        Ok(n)
    }

    /// Aplica a política de retenção. Devolve quantas mudanças foram apagadas.
    pub fn prune(&self) -> Result<usize> {
        let r = self.load_retention()?;
        let mut deleted = 0;
        if r.max_changes > 0 {
            deleted += self.trim_changes(None, r.max_changes)?;
        }
        if r.max_age_days > 0 {
            let cutoff = Utc::now() - chrono::Duration::days(r.max_age_days as i64);
            deleted += self.conn.execute(
                "DELETE FROM changes WHERE detected_at < ?1",
                params![cutoff.to_rfc3339()],
            )?;
        }
        let keep = r.max_snapshots.max(1);
        deleted += self.conn.execute(
            "DELETE FROM snapshots WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (
                        PARTITION BY watch_id ORDER BY fetched_at DESC, id DESC
                    ) AS rn FROM snapshots
                ) WHERE rn > ?1
             )",
            params![keep as i64],
        )?;
        Ok(deleted)
    }

    /// Devolve o espaço ao disco (use depois de apagar linhas).
    pub fn vacuum(&self) -> Result<()> {
        self.conn
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    pub fn load_retention(&self) -> Result<Retention> {
        let d = Retention::default();
        let get = |key: &str, default: u32| -> Result<u32> {
            Ok(self
                .get_setting(key)?
                .and_then(|v| v.parse().ok())
                .unwrap_or(default))
        };
        Ok(Retention {
            max_changes: get("retention_max_changes", d.max_changes)?,
            max_age_days: get("retention_max_age_days", d.max_age_days)?,
            max_snapshots: get("retention_max_snapshots", d.max_snapshots)?,
        })
    }

    pub fn save_retention(&self, r: &Retention) -> Result<()> {
        self.set_setting("retention_max_changes", &r.max_changes.to_string())?;
        self.set_setting("retention_max_age_days", &r.max_age_days.to_string())?;
        self.set_setting("retention_max_snapshots", &r.max_snapshots.to_string())?;
        Ok(())
    }

    /// Todas as mudanças (de um monitor ou de todos), da mais antiga à mais nova,
    /// com rótulo e URL — base da exportação do histórico.
    pub fn list_changes_for_export(
        &self,
        watch_id: Option<i64>,
    ) -> Result<Vec<(Change, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.watch_id, c.detected_at, c.diff_text, c.seen, w.label, w.url
             FROM changes c JOIN watches w ON w.id = c.watch_id
             WHERE (?1 IS NULL OR c.watch_id = ?1)
             ORDER BY c.watch_id ASC, c.detected_at ASC, c.id ASC",
        )?;
        let rows = stmt.query_map(params![watch_id], |row| {
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

    pub fn unread_count(&self) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM watches WHERE unread_change = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
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

    pub fn load_settings(&self) -> Result<AppSettings> {
        let theme = self
            .get_setting("theme")?
            .map(|v| ThemeMode::parse(&v))
            .unwrap_or(ThemeMode::Dark);
        let density = self
            .get_setting("density")?
            .map(|v| Density::parse(&v))
            .unwrap_or(Density::Normal);
        let flag =
            |key: &str| -> Result<bool> { Ok(self.get_setting(key)?.as_deref() == Some("1")) };
        Ok(AppSettings {
            theme,
            density,
            diff_context: flag("diff_context")?,
            diff_side_by_side: flag("diff_side_by_side")?,
        })
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        self.set_setting("theme", settings.theme.as_str())?;
        self.set_setting("density", settings.density.as_str())?;
        self.set_setting(
            "diff_context",
            if settings.diff_context { "1" } else { "0" },
        )?;
        self.set_setting(
            "diff_side_by_side",
            if settings.diff_side_by_side { "1" } else { "0" },
        )?;
        Ok(())
    }

    fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// Caminho do banco: `CURIO_DB` (testes/uso avançado) ou `~/.curio/curio.db`.
pub fn default_db_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("CURIO_DB").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let home = dirs_home().context("HOME não definido")?;
    Ok(home.join(".curio").join("curio.db"))
}

/// Pasta dos dados do Curió (banco, perfil do navegador, exportações).
pub fn data_dir() -> Result<PathBuf> {
    let db = default_db_path()?;
    Ok(db
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
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
    use crate::model::DEFAULT_MAX_SNAPSHOTS;

    fn spec(url: &str) -> WatchSpec {
        WatchSpec {
            url: url.into(),
            label: "ex".into(),
            interval_secs: 60,
            selector: "#main".into(),
            ignore_regex: "ts:".into(),
            ..Default::default()
        }
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn crud_and_snapshot_trim() {
        let db = Database::open_in_memory().unwrap();
        let w = db.insert_watch(&spec("https://example.com")).unwrap();
        assert_eq!(w.selector, "#main");
        assert_eq!(w.ignore_regex, "ts:");
        assert_eq!(w.mode, Mode::Html);

        db.set_paused(w.id, true).unwrap();
        let w2 = db.get_watch(w.id).unwrap().unwrap();
        assert!(w2.paused);

        for i in 0..25 {
            db.insert_snapshot(w.id, &format!("h{i}"), &format!("text {i}"))
                .unwrap();
        }
        assert_eq!(count(&db, "snapshots"), DEFAULT_MAX_SNAPSHOTS as i64);

        let mut settings = db.load_settings().unwrap();
        settings.theme = ThemeMode::Light;
        settings.density = Density::Compact;
        settings.diff_context = true;
        settings.diff_side_by_side = true;
        db.save_settings(&settings).unwrap();
        let s2 = db.load_settings().unwrap();
        assert_eq!(s2.theme, ThemeMode::Light);
        assert_eq!(s2.density, Density::Compact);
        assert!(s2.diff_context && s2.diff_side_by_side);
    }

    #[test]
    fn new_fields_roundtrip_through_update() {
        let db = Database::open_in_memory().unwrap();
        let w = db.insert_watch(&spec("https://example.com")).unwrap();
        let mut s = spec("https://api.example.com/x");
        s.mode = Mode::Json;
        s.condition = "número < 100".into();
        s.headers = "Authorization: Bearer ${T}".into();
        s.tags = "preço,api".into();
        db.update_watch(w.id, &s).unwrap();
        let w = db.get_watch(w.id).unwrap().unwrap();
        assert_eq!(WatchSpec::from_watch(&w), s);
        assert_eq!(w.tag_list().collect::<Vec<_>>(), vec!["preço", "api"]);
        assert!(w.down_since.is_none());
        db.set_down_since(w.id, Some(Utc::now())).unwrap();
        assert!(db.get_watch(w.id).unwrap().unwrap().down_since.is_some());
        db.set_down_since(w.id, None).unwrap();
        assert!(db.get_watch(w.id).unwrap().unwrap().down_since.is_none());
    }

    #[test]
    fn migrates_a_v0_3_database_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            // Esquema exato da v0.3.x (sem as colunas da 0.4).
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE watches (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL,
                    label TEXT NOT NULL DEFAULT '', interval_secs INTEGER NOT NULL,
                    created_at TEXT NOT NULL, last_checked_at TEXT,
                    last_status TEXT NOT NULL DEFAULT 'waiting', last_error TEXT,
                    unread_change INTEGER NOT NULL DEFAULT 0,
                    consecutive_failures INTEGER NOT NULL DEFAULT 0,
                    selector TEXT NOT NULL DEFAULT '', ignore_regex TEXT NOT NULL DEFAULT '',
                    paused INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE snapshots (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, watch_id INTEGER NOT NULL,
                    fetched_at TEXT NOT NULL, content_hash TEXT NOT NULL,
                    normalized_text TEXT NOT NULL
                 );
                 CREATE TABLE changes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, watch_id INTEGER NOT NULL,
                    detected_at TEXT NOT NULL, diff_text TEXT NOT NULL,
                    seen INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO watches (url, label, interval_secs, created_at, selector)
                    VALUES ('https://a.example', 'A', 600, '2026-01-01T00:00:00+00:00', '#x');
                 INSERT INTO snapshots (watch_id, fetched_at, content_hash, normalized_text)
                    VALUES (1, '2026-01-01T00:00:00+00:00', 'h', 't');
                 INSERT INTO changes (watch_id, detected_at, diff_text)
                    VALUES (1, '2026-01-02T00:00:00+00:00', '- a\n+ b');",
            )
            .unwrap();
        }
        let db = Database::open_path(&path).unwrap();
        let w = db.list_watches().unwrap().remove(0);
        assert_eq!(
            (w.url.as_str(), w.label.as_str(), w.selector.as_str()),
            ("https://a.example", "A", "#x")
        );
        assert_eq!(w.mode, Mode::Html);
        assert!(w.condition.is_empty() && w.headers.is_empty() && w.tags.is_empty());
        assert_eq!(db.latest_snapshot(1).unwrap().unwrap().content_hash, "h");
        assert_eq!(db.list_changes(1, 10).unwrap().len(), 1);
        // Migrar duas vezes é inofensivo.
        let db = Database::open_path(&path).unwrap();
        assert_eq!(db.list_watches().unwrap().len(), 1);
    }

    #[test]
    fn retention_trims_changes_by_count_per_watch() {
        let db = Database::open_in_memory().unwrap();
        let a = db.insert_watch(&spec("https://a.example")).unwrap();
        let b = db.insert_watch(&spec("https://b.example")).unwrap();
        db.save_retention(&Retention {
            max_changes: 3,
            ..Retention::default()
        })
        .unwrap();
        for i in 0..6 {
            db.insert_change(a.id, &format!("+ a{i}"), false).unwrap();
        }
        db.insert_change(b.id, "+ b0", false).unwrap();
        let a_left = db.list_changes(a.id, 100).unwrap();
        assert_eq!(a_left.len(), 3);
        assert_eq!(a_left[0].diff_text, "+ a5");
        assert_eq!(db.list_changes(b.id, 100).unwrap().len(), 1);
    }

    #[test]
    fn prune_applies_age_and_count_and_zero_means_unlimited() {
        let db = Database::open_in_memory().unwrap();
        let w = db.insert_watch(&spec("https://a.example")).unwrap();
        db.save_retention(&Retention {
            max_changes: 0,
            max_age_days: 0,
            max_snapshots: 20,
        })
        .unwrap();
        for i in 0..5 {
            db.insert_change(w.id, &format!("+ x{i}"), true).unwrap();
        }
        db.conn
            .execute(
                "UPDATE changes SET detected_at = '2020-01-01T00:00:00+00:00' WHERE id <= 2",
                [],
            )
            .unwrap();
        assert_eq!(db.prune().unwrap(), 0, "sem limites nada é apagado");
        db.save_retention(&Retention {
            max_changes: 0,
            max_age_days: 30,
            max_snapshots: 20,
        })
        .unwrap();
        assert_eq!(db.prune().unwrap(), 2);
        db.save_retention(&Retention {
            max_changes: 1,
            max_age_days: 0,
            max_snapshots: 1,
        })
        .unwrap();
        db.insert_snapshot(w.id, "h1", "t1").unwrap();
        db.insert_snapshot(w.id, "h2", "t2").unwrap();
        db.prune().unwrap();
        assert_eq!(count(&db, "changes"), 1);
        assert_eq!(count(&db, "snapshots"), 1);
        assert_eq!(
            db.latest_snapshot(w.id).unwrap().unwrap().content_hash,
            "h2"
        );
        db.vacuum().unwrap();
    }

    #[test]
    fn suppressed_change_is_recorded_as_seen() {
        let db = Database::open_in_memory().unwrap();
        let w = db.insert_watch(&spec("https://a.example")).unwrap();
        db.insert_change(w.id, "+ x", true).unwrap();
        assert!(db.list_changes(w.id, 10).unwrap()[0].seen);
    }

    #[test]
    fn retention_defaults_and_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.load_retention().unwrap(), Retention::default());
        let r = Retention {
            max_changes: 7,
            max_age_days: 9,
            max_snapshots: 2,
        };
        db.save_retention(&r).unwrap();
        assert_eq!(db.load_retention().unwrap(), r);
    }
}
