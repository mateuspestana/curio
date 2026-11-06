//! Domain models for Curió watches, snapshots, and changes.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use chrono::{DateTime, Utc};

pub const AUTHOR_SIGNATURE: &str =
    "Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda";

pub const MIN_INTERVAL_SECS: u64 = 5;
pub const FETCH_TIMEOUT_SECS: u64 = 15;
/// Páginas renderizadas por navegador demoram bem mais que um GET simples.
pub const BROWSER_TIMEOUT_SECS: u64 = 45;
/// Falhas seguidas até o monitor ser considerado fora do ar (e avisar).
pub const MAX_FETCH_ATTEMPTS: u32 = 3;
/// Backoff exponencial após falha: 60s, 120s, 240s… até o teto.
pub const RETRY_BASE_SECS: u64 = 60;
pub const MAX_BACKOFF_SECS: u64 = 3600;
/// Teto para respeitar um `Retry-After` do servidor.
pub const MAX_RETRY_AFTER_SECS: u64 = 86_400;

pub const DEFAULT_MAX_CHANGES: u32 = 500;
pub const DEFAULT_MAX_SNAPSHOTS: u32 = 20;

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

/// Como o conteúdo da URL é obtido e interpretado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// GET simples, texto visível do HTML.
    #[default]
    Html,
    /// GET simples, resposta JSON achatada em `caminho: valor`.
    Json,
    /// Página renderizada por um navegador headless (executa JavaScript).
    Js,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Json => "json",
            Self::Js => "js",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "json" => Self::Json,
            "js" | "browser" | "navegador" => Self::Js,
            _ => Self::Html,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Html => Self::Json,
            Self::Json => Self::Js,
            Self::Js => Self::Html,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Html => Self::Js,
            Self::Json => Self::Html,
            Self::Js => Self::Json,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Html => "html — texto visível da página",
            Self::Json => "json — API/JSON (seletor = caminho, ex.: items[*].price)",
            Self::Js => "js — renderiza JavaScript com Chrome/Chromium",
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
    /// Histórico: mostrar linhas de contexto ao redor das mudanças.
    pub diff_context: bool,
    /// Histórico: diff lado a lado (removido | adicionado).
    pub diff_side_by_side: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            density: Density::Normal,
            diff_context: false,
            diff_side_by_side: false,
        }
    }
}

/// Política de retenção do histórico (0 = sem limite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Máximo de mudanças guardadas por monitor.
    pub max_changes: u32,
    /// Idade máxima das mudanças, em dias.
    pub max_age_days: u32,
    /// Máximo de snapshots de texto por monitor (mínimo 1).
    pub max_snapshots: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            max_changes: DEFAULT_MAX_CHANGES,
            max_age_days: 0,
            max_snapshots: DEFAULT_MAX_SNAPSHOTS,
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
    pub mode: Mode,
    /// Regras de alerta, uma por linha (vazio = alertar em toda mudança).
    pub condition: String,
    /// Cabeçalhos HTTP `Nome: valor`, um por linha (aceita `${VAR}`).
    pub headers: String,
    /// Tags separadas por vírgula, já normalizadas.
    pub tags: String,
    /// Quando o monitor entrou em downtime (e o aviso foi dado).
    pub down_since: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_status: WatchStatus,
    pub last_error: Option<String>,
    pub unread_change: bool,
    pub consecutive_failures: u32,
    pub next_check_at: Option<DateTime<Utc>>,
}

impl Watch {
    pub fn tag_list(&self) -> impl Iterator<Item = &str> {
        self.tags.split(',').filter(|t| !t.is_empty())
    }

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
    /// O monitor foi checado; a UI relê o banco.
    WatchUpdated(i64),
    ChangeDetected {
        watch_id: i64,
        label: String,
    },
    Downtime {
        watch_id: i64,
        label: String,
        error: String,
    },
    Recovered {
        watch_id: i64,
        label: String,
    },
}
