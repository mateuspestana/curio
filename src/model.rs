//! Domain models for Curió watches, snapshots, and changes.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use chrono::{DateTime, Utc};

pub const AUTHOR_SIGNATURE: &str =
    "Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda";

pub const MAX_SNAPSHOTS_PER_WATCH: usize = 20;
pub const MIN_INTERVAL_SECS: u64 = 5;
pub const FETCH_TIMEOUT_SECS: u64 = 15;
pub const RETRY_INTERVAL_SECS: u64 = 300; // 5 minutes
pub const MAX_FETCH_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    Waiting,
    Ok,
    Changed,
    Error,
    Paused,
}

impl WatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Ok => "ok",
            Self::Changed => "changed",
            Self::Error => "error",
            Self::Paused => "paused",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "changed" => Self::Changed,
            "error" => Self::Error,
            "paused" => Self::Paused,
            _ => Self::Waiting,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            _ => Self::Dark,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Normal,
    Compact,
}

impl Density {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Compact => "compact",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "compact" => Self::Compact,
            _ => Self::Normal,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Normal => Self::Compact,
            Self::Compact => Self::Normal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub theme: ThemeMode,
    pub density: Density,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            density: Density::Normal,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Watch {
    pub id: i64,
    pub url: String,
    pub label: String,
    pub interval_secs: u64,
    pub selector: String,
    pub ignore_regex: String,
    pub paused: bool,
    pub created_at: DateTime<Utc>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_status: WatchStatus,
    pub last_error: Option<String>,
    pub unread_change: bool,
    pub consecutive_failures: u32,
    pub next_check_at: Option<DateTime<Utc>>,
}

impl Watch {
    pub fn display_name(&self) -> &str {
        if self.label.is_empty() {
            &self.url
        } else {
            &self.label
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Snapshot {
    pub id: i64,
    pub watch_id: i64,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: String,
    pub normalized_text: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Change {
    pub id: i64,
    pub watch_id: i64,
    pub detected_at: DateTime<Utc>,
    pub diff_text: String,
    pub seen: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppEvent {
    WatchUpdated(Watch),
    ChangeDetected { watch_id: i64, label: String },
    Downtime { watch_id: i64, label: String, error: String },
}
