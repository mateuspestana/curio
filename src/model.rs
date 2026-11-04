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
}

impl WatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Ok => "ok",
            Self::Changed => "changed",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "changed" => Self::Changed,
            "error" => Self::Error,
            _ => Self::Waiting,
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
    pub created_at: DateTime<Utc>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_status: WatchStatus,
    pub last_error: Option<String>,
    pub unread_change: bool,
    pub consecutive_failures: u32,
    pub next_check_at: Option<DateTime<Utc>>,
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
    Tick,
}
