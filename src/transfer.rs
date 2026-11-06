//! Exportar/importar monitores (TOML ou JSON) e exportar o histórico (CSV ou Markdown).
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::db::Database;
use crate::diff::{changed_only, count_changes};
use crate::interval::{format_interval, parse_interval};
use crate::model::{Change, Mode, Watch};
use crate::spec::{normalize_tags, WatchSpec};
use anyhow::{bail, Context, Result};
use chrono::{Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Toml,
    Json,
}

impl ConfigFormat {
    /// `.json` → JSON; qualquer outra coisa → TOML.
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some(e) if e.eq_ignore_ascii_case("json") => Self::Json,
            _ => Self::Toml,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFormat {
    Csv,
    Markdown,
}

impl HistoryFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Markdown => "md",
        }
    }
}

/// Grava o arquivo; `private` restringe a leitura ao usuário (0600 no Unix).
pub fn write_file(path: &Path, content: &str, private: bool) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .with_context(|| format!("gravar {}", path.display()))?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        // `mode` só vale na criação: garante também em arquivos que já existiam.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    file.write_all(content.as_bytes())
        .with_context(|| format!("gravar {}", path.display()))?;
    Ok(())
}

fn one() -> u32 {
    1
}

fn default_interval() -> String {
    "10m".into()
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_html(m: &str) -> bool {
    m.is_empty() || m == "html"
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportFile {
    #[serde(default = "one")]
    version: u32,
    #[serde(default, rename = "monitor")]
    monitors: Vec<MonitorConfig>,
}

/// Um monitor no arquivo: listas em vez de texto multilinha, para editar à mão.
#[derive(Debug, Default, Serialize, Deserialize)]
struct MonitorConfig {
    url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default, skip_serializing_if = "is_html")]
    mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    selector: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ignore: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    condition: Vec<String>,
    /// Modelos de cabeçalho (`Nome: ${VAR}`), nunca valores expandidos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    paused: bool,
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

impl MonitorConfig {
    fn from_watch(w: &Watch) -> Self {
        Self {
            url: w.url.clone(),
            label: w.label.clone(),
            interval: format_interval(w.interval_secs),
            mode: w.mode.as_str().to_string(),
            selector: w.selector.clone(),
            ignore: lines(&w.ignore_regex),
            condition: lines(&w.condition),
            headers: lines(&w.headers),
            tags: w.tag_list().map(String::from).collect(),
            paused: w.paused,
        }
    }

    fn into_spec(self) -> Result<WatchSpec, String> {
        let interval_secs = parse_interval(&self.interval).map_err(|e| e.to_string())?;
        let spec = WatchSpec {
            url: self.url,
            label: self.label,
            interval_secs,
            mode: Mode::parse(&self.mode),
            selector: self.selector,
            ignore_regex: self.ignore.join("\n"),
            condition: self.condition.join("\n"),
            headers: self.headers.join("\n"),
            tags: normalize_tags(&self.tags.join(",")),
        }
        .normalized();
        spec.validate().map_err(|e| e.message)?;
        Ok(spec)
    }
}

/// Serializa os monitores (configuração apenas: sem snapshots nem histórico).
pub fn export_watches(watches: &[Watch], format: ConfigFormat) -> Result<String> {
    let file = ExportFile {
        version: 1,
        monitors: watches.iter().map(MonitorConfig::from_watch).collect(),
    };
    Ok(match format {
        ConfigFormat::Toml => toml::to_string_pretty(&file).context("serializar TOML")?,
        ConfigFormat::Json => {
            let mut s = serde_json::to_string_pretty(&file).context("serializar JSON")?;
            s.push('\n');
            s
        }
    })
}

/// Avisos sobre segredos gravados em texto puro no arquivo exportado.
pub fn export_warnings(watches: &[Watch]) -> Vec<String> {
    let mut out = Vec::new();
    for w in watches {
        for line in w.headers.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some((name, value)) = line.split_once(':') {
                if !value.contains("${") {
                    out.push(format!(
                        "{}: o cabeçalho {} tem valor literal e foi exportado como está; prefira ${{VARIAVEL}}",
                        w.display_name(),
                        name.trim()
                    ));
                }
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub added: usize,
    pub duplicates: usize,
    /// `(identificação, motivo)` de cada monitor rejeitado.
    pub invalid: Vec<(String, String)>,
}

/// Importa monitores. Já existente = mesma URL, seletor e modo (é ignorado).
pub fn import_watches(db: &Database, text: &str, format: ConfigFormat) -> Result<ImportReport> {
    let file: ExportFile = match format {
        ConfigFormat::Toml => toml::from_str(text).context("ler TOML")?,
        ConfigFormat::Json => serde_json::from_str(text).context("ler JSON")?,
    };
    if file.version != 1 {
        bail!(
            "versão {} do arquivo não suportada (esperado 1)",
            file.version
        );
    }

    let mut known: Vec<(String, String, Mode)> = db
        .list_watches()?
        .into_iter()
        .map(|w| (w.url, w.selector, w.mode))
        .collect();
    let mut report = ImportReport::default();

    for (i, m) in file.monitors.into_iter().enumerate() {
        let name = if m.label.is_empty() {
            format!("#{} {}", i + 1, m.url)
        } else {
            format!("#{} {}", i + 1, m.label)
        };
        let paused = m.paused;
        let spec = match m.into_spec() {
            Ok(s) => s,
            Err(reason) => {
                report.invalid.push((name, reason));
                continue;
            }
        };
        let key = (spec.url.clone(), spec.selector.clone(), spec.mode);
        if known.contains(&key) {
            report.duplicates += 1;
            continue;
        }
        let w = db.insert_watch(&spec)?;
        if paused {
            db.set_paused(w.id, true)?;
        }
        known.push(key);
        report.added += 1;
    }
    Ok(report)
}

// ------------------------------------------------------------------- histórico

fn csv_cell(value: &str) -> String {
    // Texto vindo de sites é não confiável: neutraliza fórmulas de planilha.
    let guarded = match value.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{value}"),
        _ => value.to_string(),
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn local_time(change: &Change) -> String {
    change
        .detected_at
        .with_timezone(&Local)
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// Cerca de código mais longa que qualquer sequência de crases no conteúdo.
fn fence_for(content: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

pub fn export_history(rows: &[(Change, String, String)], format: HistoryFormat) -> String {
    match format {
        HistoryFormat::Csv => {
            let mut out =
                String::from("id,monitor,url,detectado_em,visto,adicionadas,removidas,diff\r\n");
            for (c, label, url) in rows {
                let (added, removed) = count_changes(&c.diff_text);
                let diff = changed_only(&c.diff_text);
                let cells = [
                    c.id.to_string(),
                    csv_cell(if label.is_empty() { url } else { label }),
                    csv_cell(url),
                    local_time(c),
                    (if c.seen { "sim" } else { "não" }).to_string(),
                    added.to_string(),
                    removed.to_string(),
                    csv_cell(diff.trim_end()),
                ];
                out.push_str(&cells.join(","));
                out.push_str("\r\n");
            }
            out
        }
        HistoryFormat::Markdown => {
            let mut out = String::from("# Histórico de mudanças — Curió\n");
            let mut current: Option<i64> = None;
            for (c, label, url) in rows {
                if current != Some(c.watch_id) {
                    current = Some(c.watch_id);
                    let name = if label.is_empty() { url } else { label };
                    out.push_str(&format!("\n## {}\n\n<{url}>\n", name.replace('\n', " ")));
                }
                let (added, removed) = count_changes(&c.diff_text);
                let diff = c.diff_text.trim_end();
                let fence = fence_for(diff);
                out.push_str(&format!(
                    "\n### {} (+{added} −{removed})\n\n{fence}diff\n{diff}\n{fence}\n",
                    local_time(c)
                ));
            }
            if rows.is_empty() {
                out.push_str("\nNenhuma mudança registrada.\n");
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn spec(url: &str) -> WatchSpec {
        WatchSpec {
            url: url.into(),
            interval_secs: 600,
            ..Default::default()
        }
    }

    fn rich_spec() -> WatchSpec {
        WatchSpec {
            url: "https://api.example.com/x".into(),
            label: "Preço".into(),
            interval_secs: 90,
            mode: Mode::Json,
            selector: "items[*].price".into(),
            ignore_regex: "ts:\ncache".into(),
            condition: "número < 100\ncontém: promo".into(),
            headers: "Authorization: Bearer ${TOKEN}\nCookie: a=1; b=2".into(),
            tags: "preço,api".into(),
        }
    }

    fn change(id: i64, watch_id: i64, diff: &str, seen: bool) -> Change {
        Change {
            id,
            watch_id,
            detected_at: Utc::now(),
            diff_text: diff.into(),
            seen,
        }
    }

    #[test]
    fn export_import_roundtrip_toml_and_json() {
        for format in [ConfigFormat::Toml, ConfigFormat::Json] {
            let src = Database::open_in_memory().unwrap();
            src.insert_watch(&rich_spec()).unwrap();
            let plain = src.insert_watch(&spec("https://a.example")).unwrap();
            src.set_paused(plain.id, true).unwrap();
            let text = export_watches(&src.list_watches().unwrap(), format).unwrap();

            let dst = Database::open_in_memory().unwrap();
            let report = import_watches(&dst, &text, format).unwrap();
            assert_eq!(
                (report.added, report.duplicates, report.invalid.len()),
                (2, 0, 0)
            );

            let got = dst.list_watches().unwrap();
            assert_eq!(
                WatchSpec::from_watch(&got[0]),
                rich_spec(),
                "{format:?}\n{text}"
            );
            assert_eq!(WatchSpec::from_watch(&got[1]), spec("https://a.example"));
            assert!(got[1].paused && !got[0].paused);
        }
    }

    #[test]
    fn toml_export_is_hand_editable() {
        let db = Database::open_in_memory().unwrap();
        db.insert_watch(&rich_spec()).unwrap();
        let text = export_watches(&db.list_watches().unwrap(), ConfigFormat::Toml).unwrap();
        assert!(text.contains("[[monitor]]"), "{text}");
        assert!(text.contains("interval = \"90s\""), "{text}");
        assert!(
            text.contains("${TOKEN}"),
            "o modelo do segredo é preservado"
        );
        // Padrões não poluem o arquivo.
        let db = Database::open_in_memory().unwrap();
        db.insert_watch(&spec("https://a.example")).unwrap();
        let text = export_watches(&db.list_watches().unwrap(), ConfigFormat::Toml).unwrap();
        assert!(
            !text.contains("mode") && !text.contains("headers"),
            "{text}"
        );
    }

    #[test]
    fn import_skips_duplicates_and_reports_invalid() {
        let db = Database::open_in_memory().unwrap();
        db.insert_watch(&spec("https://a.example")).unwrap();
        let text = r#"
            [[monitor]]
            url = "https://a.example"
            interval = "1h"

            [[monitor]]
            url = "https://b.example"

            [[monitor]]
            url = "https://b.example"

            [[monitor]]
            url = "ftp://ruim"

            [[monitor]]
            url = "https://c.example"
            interval = "xx"

            [[monitor]]
            url = "https://d.example"
            condition = ["bobagem"]
        "#;
        let r = import_watches(&db, text, ConfigFormat::Toml).unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.duplicates, 2, "a.example existente + b.example repetido");
        assert_eq!(r.invalid.len(), 3);
        assert!(r.invalid[0].0.contains("ftp://ruim"));
        assert_eq!(db.list_watches().unwrap().len(), 2);
        let imported = &db.list_watches().unwrap()[1];
        assert_eq!(imported.interval_secs, 600, "padrão de 10m");
    }

    #[test]
    fn import_rejects_garbage_and_unknown_version() {
        let db = Database::open_in_memory().unwrap();
        assert!(import_watches(&db, "isso = não é = toml", ConfigFormat::Toml).is_err());
        assert!(import_watches(&db, "version = 9", ConfigFormat::Toml).is_err());
        assert!(import_watches(&db, "{", ConfigFormat::Json).is_err());
    }

    #[test]
    fn format_is_inferred_from_extension() {
        assert_eq!(
            ConfigFormat::from_path(Path::new("a.json")),
            ConfigFormat::Json
        );
        assert_eq!(
            ConfigFormat::from_path(Path::new("a.JSON")),
            ConfigFormat::Json
        );
        assert_eq!(
            ConfigFormat::from_path(Path::new("a.toml")),
            ConfigFormat::Toml
        );
        assert_eq!(ConfigFormat::from_path(Path::new("a")), ConfigFormat::Toml);
    }

    #[test]
    fn warns_about_literal_header_secrets_only() {
        let db = Database::open_in_memory().unwrap();
        let mut s = spec("https://a.example");
        s.headers = "Authorization: Bearer ${T}\nX-Api-Key: abc123".into();
        db.insert_watch(&s).unwrap();
        let w = export_warnings(&db.list_watches().unwrap());
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("X-Api-Key") && !w[0].contains("abc123"));
    }

    #[test]
    fn csv_quotes_per_rfc4180_and_blocks_formulas() {
        let rows = vec![(
            change(1, 1, "  ctx\n- a, \"b\"\n+ c\n", false),
            "Meu \"site\"".to_string(),
            "https://x.example/?a=1,2".to_string(),
        )];
        let csv = export_history(&rows, HistoryFormat::Csv);
        let mut lines = csv.split("\r\n");
        assert_eq!(
            lines.next().unwrap(),
            "id,monitor,url,detectado_em,visto,adicionadas,removidas,diff"
        );
        let record = csv.split("\r\n").nth(1).unwrap_or_default();
        assert!(record.starts_with("1,\"Meu \"\"site\"\"\",\"https://x.example/?a=1,2\","));
        assert!(csv.contains(",não,1,1,"));
        // Diff sem contexto, multilinha entre aspas, protegido contra fórmula (começa com '-').
        assert!(csv.contains("\"'- a, \"\"b\"\"\n+ c\"\r\n"), "{csv}");
        assert!(!csv.contains("ctx"));
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("normal"), "normal");
    }

    #[test]
    fn markdown_groups_by_monitor_and_survives_backticks() {
        let rows = vec![
            (
                change(1, 1, "  ctx\n- a\n+ b\n", true),
                "Site A".into(),
                "https://a".into(),
            ),
            (
                change(2, 1, "- ```x```\n+ y\n", false),
                "Site A".into(),
                "https://a".into(),
            ),
            (
                change(3, 2, "+ novo\n", false),
                String::new(),
                "https://b".into(),
            ),
        ];
        let md = export_history(&rows, HistoryFormat::Markdown);
        assert_eq!(md.matches("\n## ").count(), 2);
        assert!(md.contains("## Site A") && md.contains("## https://b"));
        assert!(md.contains("(+1 −1)"));
        assert!(md.contains("```diff\n  ctx\n- a\n+ b\n```\n"));
        assert!(md.contains("````diff\n- ```x```\n+ y\n````\n"), "{md}");
        assert!(export_history(&[], HistoryFormat::Markdown).contains("Nenhuma mudança"));
    }

    #[test]
    fn export_history_query_orders_chronologically() {
        let db = Database::open_in_memory().unwrap();
        let a = db.insert_watch(&spec("https://a.example")).unwrap();
        let b = db.insert_watch(&spec("https://b.example")).unwrap();
        db.insert_change(b.id, "+ b1", false).unwrap();
        db.insert_change(a.id, "+ a1", false).unwrap();
        db.insert_change(a.id, "+ a2", false).unwrap();
        let all = db.list_changes_for_export(None).unwrap();
        let order: Vec<_> = all.iter().map(|(c, _, _)| c.diff_text.as_str()).collect();
        assert_eq!(order, ["+ a1", "+ a2", "+ b1"]);
        assert_eq!(db.list_changes_for_export(Some(b.id)).unwrap().len(), 1);
    }
}
