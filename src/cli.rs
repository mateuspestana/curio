//! Linha de comando: exportar/importar, histórico, retenção e limpeza.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::db::Database;
use crate::model::Retention;
use crate::transfer::{
    export_history, export_warnings, export_watches, import_watches, write_file, ConfigFormat,
    HistoryFormat,
};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "curio",
    version,
    about = "Curió — monitora mudanças em URLs (sem subcomando, abre a TUI)",
    after_help = "Dados em ~/.curio/curio.db (ou no arquivo indicado em CURIO_DB)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Exporta os monitores (TOML ou JSON; padrão: TOML na saída padrão)
    Export {
        /// Arquivo de saída; a extensão .json escolhe JSON
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Força o formato (ignora a extensão)
        #[arg(short, long, value_enum)]
        format: Option<ConfigFormatArg>,
    },
    /// Importa monitores de um arquivo TOML/JSON (ignora os que já existem)
    Import {
        /// Arquivo gerado por `curio export` (ou escrito à mão)
        file: PathBuf,
    },
    /// Exporta o histórico de mudanças (CSV ou Markdown)
    History {
        /// Só este monitor (id)
        #[arg(short, long)]
        watch: Option<i64>,
        #[arg(short, long, value_enum, default_value = "csv")]
        format: HistoryFormatArg,
        /// Arquivo de saída (padrão: saída padrão)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Mostra ou altera a retenção do histórico
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Aplica a retenção agora e compacta o banco
    Prune,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Define um valor: max-changes, max-days ou max-snapshots (0 = sem limite)
    Set { key: String, value: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigFormatArg {
    Toml,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum HistoryFormatArg {
    Csv,
    Md,
}

impl From<HistoryFormatArg> for HistoryFormat {
    fn from(f: HistoryFormatArg) -> Self {
        match f {
            HistoryFormatArg::Csv => Self::Csv,
            HistoryFormatArg::Md => Self::Markdown,
        }
    }
}

/// `private`: o conteúdo pode ter segredos (cabeçalhos) — arquivo só do usuário.
fn write_or_print(output: Option<&Path>, content: &str, private: bool) -> Result<()> {
    match output {
        Some(path) => {
            write_file(path, content, private)?;
            eprintln!("Gravado em {}", path.display());
        }
        None => print!("{content}"),
    }
    Ok(())
}

pub fn run(command: Command, db_path: &Path) -> Result<()> {
    let db = Database::open_path(db_path)?;
    match command {
        Command::Export { output, format } => {
            let format = match (format, &output) {
                (Some(ConfigFormatArg::Toml), _) => ConfigFormat::Toml,
                (Some(ConfigFormatArg::Json), _) => ConfigFormat::Json,
                (None, Some(p)) => ConfigFormat::from_path(p),
                (None, None) => ConfigFormat::Toml,
            };
            let watches = db.list_watches()?;
            for w in export_warnings(&watches) {
                eprintln!("aviso: {w}");
            }
            write_or_print(output.as_deref(), &export_watches(&watches, format)?, true)?;
        }
        Command::Import { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("ler {}", file.display()))?;
            let report = import_watches(&db, &text, ConfigFormat::from_path(&file))?;
            println!(
                "{} importado(s), {} já existia(m), {} inválido(s).",
                report.added,
                report.duplicates,
                report.invalid.len()
            );
            for (name, reason) in &report.invalid {
                eprintln!("  ✗ {name}: {reason}");
            }
            if !report.invalid.is_empty() {
                bail!("{} monitor(es) não importado(s)", report.invalid.len());
            }
        }
        Command::History {
            watch,
            format,
            output,
        } => {
            if let Some(id) = watch {
                if db.get_watch(id)?.is_none() {
                    bail!("monitor {id} não existe");
                }
            }
            let rows = db.list_changes_for_export(watch)?;
            write_or_print(
                output.as_deref(),
                &export_history(&rows, format.into()),
                false,
            )?;
        }
        Command::Config { action } => {
            let mut r = db.load_retention()?;
            if let Some(ConfigAction::Set { key, value }) = action {
                set_retention(&mut r, &key, &value)?;
                db.save_retention(&r)?;
                println!("Salvo.");
            }
            print_retention(&r);
        }
        Command::Prune => {
            let deleted = db.prune()?;
            if deleted > 0 {
                db.vacuum()?;
            }
            println!("{deleted} registro(s) removido(s).");
        }
    }
    Ok(())
}

fn set_retention(r: &mut Retention, key: &str, value: &str) -> Result<()> {
    let n: u32 = value
        .parse()
        .with_context(|| format!("valor inválido \"{value}\" (use um inteiro ≥ 0)"))?;
    match key {
        "max-changes" => r.max_changes = n,
        "max-days" => r.max_age_days = n,
        "max-snapshots" => {
            if n == 0 {
                bail!("max-snapshots precisa ser ≥ 1 (o último snapshot é a base do diff)");
            }
            r.max_snapshots = n;
        }
        other => {
            bail!("chave desconhecida \"{other}\" (use max-changes, max-days ou max-snapshots)")
        }
    }
    Ok(())
}

fn print_retention(r: &Retention) {
    let limit = |n: u32, unit: &str| {
        if n == 0 {
            "sem limite".to_string()
        } else {
            format!("{n} {unit}")
        }
    };
    println!(
        "max-changes   {}  (mudanças por monitor)",
        limit(r.max_changes, "")
    );
    println!(
        "max-days      {}  (idade máxima das mudanças)",
        limit(r.max_age_days, "dias")
    );
    println!(
        "max-snapshots {}  (textos guardados por monitor)",
        r.max_snapshots
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_subcommands() {
        let c = Cli::try_parse_from(["curio", "history", "-w", "3", "-f", "md"]).unwrap();
        assert!(matches!(
            c.command,
            Some(Command::History {
                watch: Some(3),
                format: HistoryFormatArg::Md,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["curio"]).unwrap().command.is_none());
        assert!(Cli::try_parse_from(["curio", "config", "set", "max-days", "30"]).is_ok());
    }

    #[test]
    fn set_retention_validates() {
        let mut r = Retention::default();
        set_retention(&mut r, "max-changes", "10").unwrap();
        set_retention(&mut r, "max-days", "0").unwrap();
        assert_eq!((r.max_changes, r.max_age_days), (10, 0));
        assert!(set_retention(&mut r, "max-snapshots", "0").is_err());
        assert!(set_retention(&mut r, "xx", "1").is_err());
        assert!(set_retention(&mut r, "max-days", "-1").is_err());
    }
}
