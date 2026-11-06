//! Diffs de texto para o histórico: por item, com contexto opcional e lado a lado.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use similar::{ChangeTag, TextDiff};

const MAX_DIFF_CHARS: usize = 50_000;

/// Linhas de contexto gravadas ao redor de cada mudança (a UI pode escondê-las).
pub const CONTEXT_LINES: usize = 2;

/// Separador entre trechos distantes (começa com espaço, como o contexto).
const GAP: &str = "  ⋯";

/// Diff por linha. Cada linha do texto normalizado é um item.
///
/// Formato: `- removido`, `+ adicionado`, `  contexto` (dois espaços) e `  ⋯`
/// entre trechos distantes. Com `context = 0` só sobram as linhas alteradas
/// (e os separadores).
pub fn diff_with_context(old: &str, new: &str, context: usize) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    'groups: for (i, group) in diff.grouped_ops(context).iter().enumerate() {
        if i > 0 {
            out.push_str(GAP);
            out.push('\n');
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                    ChangeTag::Equal => ' ',
                };
                out.push(sign);
                out.push(' ');
                out.push_str(change.to_string().trim_end_matches('\n'));
                out.push('\n');

                if out.len() >= MAX_DIFF_CHARS {
                    out.push_str("\n… (diff truncado)\n");
                    break 'groups;
                }
            }
        }
    }

    if out.is_empty() {
        "(sem diferenças textuais)".to_string()
    } else {
        out
    }
}

/// Só as linhas alteradas: tira contexto e separadores de um diff gravado.
pub fn changed_only(diff: &str) -> String {
    let kept: Vec<&str> = diff.lines().filter(|l| !l.starts_with(' ')).collect();
    let mut out = kept.join("\n");
    if diff.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn count_changes(diff: &str) -> (usize, usize) {
    let added = diff.lines().filter(|l| l.starts_with("+ ")).count();
    let removed = diff.lines().filter(|l| l.starts_with("- ")).count();
    (added, removed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Change,
    Context,
    Gap,
    Note,
}

/// Uma linha da visão lado a lado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideRow {
    pub left: Option<String>,
    pub right: Option<String>,
    pub kind: RowKind,
}

/// Converte um diff gravado em linhas `removido | adicionado`.
///
/// Blocos de `-` seguidos de `+` são pareados linha a linha; o excedente fica
/// com o outro lado vazio. Contexto aparece nos dois lados quando pedido.
pub fn side_by_side(diff: &str, show_context: bool) -> Vec<SideRow> {
    fn flush(removed: &mut Vec<String>, added: &mut Vec<String>, rows: &mut Vec<SideRow>) {
        let n = removed.len().max(added.len());
        for i in 0..n {
            rows.push(SideRow {
                left: removed.get(i).cloned(),
                right: added.get(i).cloned(),
                kind: RowKind::Change,
            });
        }
        removed.clear();
        added.clear();
    }

    let mut rows = Vec::new();
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for line in diff.lines() {
        if let Some(t) = line.strip_prefix("- ") {
            // Um `-` depois de `+` começa um novo bloco.
            if !added.is_empty() {
                flush(&mut removed, &mut added, &mut rows);
            }
            removed.push(t.to_string());
        } else if let Some(t) = line.strip_prefix("+ ") {
            added.push(t.to_string());
        } else if line == GAP {
            flush(&mut removed, &mut added, &mut rows);
            if show_context {
                rows.push(SideRow {
                    left: None,
                    right: None,
                    kind: RowKind::Gap,
                });
            }
        } else if let Some(t) = line.strip_prefix("  ") {
            flush(&mut removed, &mut added, &mut rows);
            if show_context {
                rows.push(SideRow {
                    left: Some(t.to_string()),
                    right: Some(t.to_string()),
                    kind: RowKind::Context,
                });
            }
        } else if !line.is_empty() {
            flush(&mut removed, &mut added, &mut rows);
            rows.push(SideRow {
                left: Some(line.to_string()),
                right: None,
                kind: RowKind::Note,
            });
        }
    }
    flush(&mut removed, &mut added, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "a\nb\nc\nd\ne\nf\ng\nh\n";
    const NEW: &str = "a\nb\nC\nd\ne\nf\ng\nH\n";

    #[test]
    fn shows_only_changed_items_without_context() {
        let d = diff_with_context("a / x\nb / y\nc / z\n", "a / x\nb / w\nc / z\n", 0);
        assert_eq!(d, "- b / y\n+ b / w\n");
    }

    #[test]
    fn includes_context_and_gaps() {
        let d = diff_with_context(OLD, NEW, 1);
        assert_eq!(
            d, "  b\n- c\n+ C\n  d\n  ⋯\n  g\n- h\n+ H\n",
            "contexto de 1 linha, separador entre trechos distantes"
        );
    }

    #[test]
    fn changed_only_strips_context_and_gaps() {
        let d = diff_with_context(OLD, NEW, CONTEXT_LINES);
        assert_eq!(changed_only(&d), "- c\n+ C\n- h\n+ H\n");
        assert_eq!(changed_only("- a\n+ b\n"), "- a\n+ b\n");
        // Diffs antigos (sem contexto) passam intactos.
        assert_eq!(
            changed_only("(sem diferenças textuais)"),
            "(sem diferenças textuais)"
        );
    }

    #[test]
    fn identical_texts_report_no_difference() {
        assert_eq!(
            diff_with_context("a\n", "a\n", 2),
            "(sem diferenças textuais)"
        );
    }

    #[test]
    fn counts_added_and_removed() {
        assert_eq!(count_changes("- a\n+ b\n+ c\n  ctx\n"), (2, 1));
    }

    #[test]
    fn side_by_side_pairs_blocks_and_keeps_extras() {
        let rows = side_by_side("- a\n- b\n+ x\n", false);
        assert_eq!(
            rows,
            vec![
                SideRow {
                    left: Some("a".into()),
                    right: Some("x".into()),
                    kind: RowKind::Change
                },
                SideRow {
                    left: Some("b".into()),
                    right: None,
                    kind: RowKind::Change
                },
            ]
        );
        let only_add = side_by_side("+ novo\n", false);
        assert_eq!(only_add[0].left, None);
        assert_eq!(only_add[0].right.as_deref(), Some("novo"));
    }

    #[test]
    fn side_by_side_context_toggle() {
        let d = diff_with_context(OLD, NEW, 1);
        let with = side_by_side(&d, true);
        let without = side_by_side(&d, false);
        assert!(with.iter().any(|r| r.kind == RowKind::Context));
        assert!(with.iter().any(|r| r.kind == RowKind::Gap));
        assert!(without.iter().all(|r| r.kind == RowKind::Change));
        assert_eq!(without.len(), 2);
    }

    #[test]
    fn side_by_side_separates_consecutive_blocks() {
        let rows = side_by_side("- a\n+ b\n- c\n+ d\n", false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].left.as_deref(), Some("c"));
        assert_eq!(rows[1].right.as_deref(), Some("d"));
    }
}
